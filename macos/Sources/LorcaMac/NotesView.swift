import AppKit
import SwiftUI

struct NotesView: View {
    @ObservedObject var store: NotesStore
    @ObservedObject var following: NotesFollowing
    @ObservedObject var chat: ChatStore

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("Notes").font(ShellTypography.serif(26).weight(.semibold))
                Spacer()
                Button { store.create() } label: {
                    Label("New note", systemImage: "square.and.pencil")
                }
                .controlSize(.small)
                .disabled(store.isReadOnly)
                .accessibilityIdentifier("notes-new")
            }
            .padding(.top, 14).padding(.bottom, 28)

            NotesTextEditor(store: store, following: following)
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let error = store.error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled).padding(.vertical, 8)
            }
            HStack(spacing: 6) {
                Menu {
                    Button(store.previewsPaused ? "Resume model following" : "Pause model following") {
                        store.previewsPaused.toggle()
                    }
                    Text(following.status)
                    if following.partialContext { Text("Following the passage near your cursor") }
                    Menu("Model: \(chat.selectedModel)") {
                        ForEach(chat.modelChoices, id: \.self) { choice in
                            Button(choice.model) { Task { await chat.selectModel(choice) } }
                        }
                    }
                    Button("Refresh model work") { following.refresh() }
                } label: {
                    Label("Model following", systemImage: store.previewsPaused ? "pause.circle" : (following.working ? "circle.dotted" : "sparkles"))
                }
                .menuStyle(.borderlessButton).fixedSize()
                .help(following.status)
                Spacer()
                Text("\(store.wordCount) \(store.wordCount == 1 ? "word" : "words")")
            }
            .font(.system(size: 10)).foregroundStyle(.secondary)
            .padding(.top, 12).padding(.bottom, 18)
        }
        .padding(.horizontal, 32)
        .onAppear { following.setVisible(true) }
        .onDisappear { following.setVisible(false) }
    }
}

struct NotesHistory: View {
    @ObservedObject var store: NotesStore

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(store.notes) { note in
                Button { store.select(note.id) } label: {
                    HStack(spacing: 0) {
                        Text(note.title)
                        Spacer(minLength: 0)
                    }
                        .font(.system(size: 11)).lineLimit(1)
                        .foregroundStyle(note.id == store.selectedID ? .primary : .secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.leading, 39).padding(.trailing, 10).padding(.vertical, 7)
                        .background(note.id == store.selectedID ? Color.primary.opacity(0.05) : .clear,
                                    in: RoundedRectangle(cornerRadius: 6))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("notes-history-\(note.id)")
            }
        }
        .padding(.bottom, 10)
    }
}

struct NotesTextEditor: NSViewRepresentable {
    @ObservedObject var store: NotesStore
    @ObservedObject var following: NotesFollowing
    final class Coordinator { var displayedID: UUID?; var focusRequest: UUID? }
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        let note = store.current, coordinator = context.coordinator
        let document = store.document(for: note.id), editor = document.editor
        let changed = coordinator.displayedID != note.id
        if changed {
            coordinator.displayedID = note.id
            scroll.documentView = editor
            editor.frame.size.width = scroll.contentSize.width
        }
        document.synchronize(note)
        editor.isEditable = !store.isReadOnly
        editor.previewsPaused = store.previewsPaused
        if changed || coordinator.focusRequest != store.focusRequest {
            coordinator.focusRequest = store.focusRequest
            editor.setSelectedRange(note.selection)
            editor.scrollRangeToVisible(note.selection)
            editor.focusOnAttach = !store.isReadOnly
            editor.focusIfReady()
        }
        editor.showWorking(following.working && !store.previewsPaused,
            document: following.workingDocument, at: following.workingLocation)
        editor.onSectionRetry = { [weak following] source in following?.retry(source: source) }
        editor.showInline(store.previewsPaused ? [] : following.inline)
    }
}

final class NotesNativeTextView: NSTextView {
    let noteUndo = UndoManager()
    override var undoManager: UndoManager? { noteUndo }
    var modelMetadata: [UUID: NotesModelBlock] = [:]
    var userEditMetadata: [UUID: NotesUserEdit] = [:]
    var changingChecklist = false
    var onImagePromises: (([NSFilePromiseReceiver], NSRange) -> Void)?
    var onImagesPasted: (([ImageInput]) -> Void)?
    var onCalendarOpen: ((URL) -> Void)?
    var formattedText: String?
    var focusOnAttach = false
    var documentID: UUID?
    var previewsPaused = false
    var modelDriven = false
    var modelLabel = ""
    var onProposalDismiss: ((NotesProposal) -> Void)?
    private var modelProposal: NotesProposal?
    private var pendingSuggestion: DispatchWorkItem?
    private var suggestion: NotesTableSuggestion?
    private var suggestionSnapshot = ""
    private var dismissedText: String?
    private var marker: NSButton?
    private var preview: NSPopover?
    private var workingCue: NotesWorkingCue?
    private var pendingWorkingCue: DispatchWorkItem?
    private var workingToken: UUID?
    private var workingDocument = ""
    private var workingLocation = 0
    var onSectionRead: ((String) -> Void)?
    var onSectionRetry: ((String) -> Void)?
    private var inlineStates: [NotesInlineState] = []
    private var inlineViews: [NotesInlineActivity] = []
    private static let originalParagraph = NSAttributedString.Key("lorca.inline.originalParagraph")

    override func mouseDown(with event: NSEvent) {
        if toggleChecklist(at: convert(event.locationInWindow, from: nil)) { return }
        if let manager = layoutManager, let container = textContainer {
            let point = convert(event.locationInWindow, from: nil)
            let local = NSPoint(x: point.x - textContainerOrigin.x, y: point.y - textContainerOrigin.y)
            let index = manager.characterIndex(for: local, in: container, fractionOfDistanceBetweenInsertionPoints: nil)
            if index < (string as NSString).length,
               let url = textStorage?.attribute(.link, at: index, effectiveRange: nil) as? URL, url.isFileURL, url.pathExtension == "ics" {
                let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: index, length: 1), actualCharacterRange: nil)
                if manager.boundingRect(forGlyphRange: glyphs, in: container).contains(local) { onCalendarOpen?(url); return }
            }
        }
        super.mouseDown(with: event)
    }

    @objc(undo:) func undoNote(_ sender: Any?) {
        breakUndoCoalescing()
        if noteUndo.canUndo { noteUndo.undo() }
    }
    @objc(redo:) func redoNote(_ sender: Any?) {
        if noteUndo.canRedo { noteUndo.redo() }
    }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.modifierFlags.contains(.command), !event.modifierFlags.contains(.option), !event.modifierFlags.contains(.control), event.charactersIgnoringModifiers?.lowercased() == "z" {
            if event.modifierFlags.contains(.shift) { redoNote(nil) } else { undoNote(nil) }
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation { acceptsImageDrop(sender.draggingPasteboard) ? .copy : super.draggingEntered(sender) }
    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation { acceptsImageDrop(sender.draggingPasteboard) ? .copy : super.draggingUpdated(sender) }
    override func prepareForDragOperation(_ sender: NSDraggingInfo) -> Bool { acceptsImageDrop(sender.draggingPasteboard) || super.prepareForDragOperation(sender) }
    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        acceptImageDrop(sender.draggingPasteboard, at: convert(sender.draggingLocation, from: nil)) || super.performDragOperation(sender)
    }

    override func paste(_ sender: Any?) {
        let images = ClipboardImages.read(.general)
        if images.isEmpty { super.paste(sender) } else { onImagesPasted?(images) }
    }
    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if item.action == NSSelectorFromString("undo:") { return noteUndo.canUndo }
        if item.action == NSSelectorFromString("redo:") { return noteUndo.canRedo }
        if item.action == #selector(paste(_:)), !ClipboardImages.read(.general).isEmpty { return isEditable }
        return super.validateUserInterfaceItem(item)
    }
    override func draw(_ dirtyRect: NSRect) {
        drawModelBlocks(in: dirtyRect)
        super.draw(dirtyRect)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        focusIfReady()
        if window == nil { resetSuggestions() }
    }

    override func layout() {
        super.layout()
        positionMarker()
        positionWorkingCue()
        positionInline()
    }

    func showInline(_ states: [NotesInlineState]) {
        guard states != inlineStates else { return }
        let scroll = enclosingScrollView
        let origin = scroll?.contentView.bounds.origin
        let writing = window?.firstResponder === self
        let before = writing ? caretLineY().flatMap { y in
            (visibleRect.minY...visibleRect.maxY).contains(y) ? y : nil
        } : nil
        inlineStates = states
        inlineViews.forEach { $0.removeFromSuperview() }
        inlineViews = states.map { state in
            let view = NotesInlineActivity(state: state)
            view.perform = { [weak self] in
                guard let self else { return }
                if !state.busy { self.onSectionRetry?(state.source) }
            }
            addSubview(view)
            return view
        }
        format()
        positionInline()
        if let scroll, let origin, let before, let after = caretLineY(), writing {
            // Keep the line being typed at the same visual position when an
            // earlier result expands. Selection and focus never change.
            scroll.contentView.scroll(to: NSPoint(x: origin.x, y: max(0, origin.y + after - before)))
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        needsLayout = true
    }

    func caretLineY() -> CGFloat? {
        guard !string.isEmpty, let manager = layoutManager, let container = textContainer else { return nil }
        manager.ensureLayout(for: container)
        let index = min(selectedRange().location, (string as NSString).length - 1)
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: index, length: 1), actualCharacterRange: nil)
        return manager.boundingRect(forGlyphRange: glyphs, in: container).minY
    }

    private func decorateInline() {
        guard let storage = textStorage else { return }
        for (state, view) in zip(inlineStates, inlineViews) {
            guard let passage = authoredPassages.first(where: { $0.source == state.source }),
                  !intersectsTable(passage.range) else { view.isHidden = true; continue }
            let paragraph = (string as NSString).paragraphRange(for: NSRange(location: NSMaxRange(passage.range) - 1, length: 1))
            let original = storage.attribute(.paragraphStyle, at: paragraph.location, effectiveRange: nil) as? NSParagraphStyle ?? NSParagraphStyle.default
            let style = original.mutableCopy() as! NSMutableParagraphStyle
            style.paragraphSpacing += view.cardHeight + 10
            storage.addAttribute(Self.originalParagraph, value: original, range: paragraph)
            storage.addAttribute(.paragraphStyle, value: style, range: paragraph)
            view.isHidden = false
        }
    }

    private func positionInline() {
        guard let manager = layoutManager, let container = textContainer else { return }
        manager.ensureLayout(for: container)
        var bottom = manager.usedRect(for: container).maxY + textContainerOrigin.y
        for (state, view) in zip(inlineStates, inlineViews) where !view.isHidden {
            guard let passage = authoredPassages.first(where: { $0.source == state.source }) else { view.isHidden = true; continue }
            let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: NSMaxRange(passage.range) - 1, length: 1), actualCharacterRange: nil)
            let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
            view.frame = NSRect(x: textContainerOrigin.x, y: rect.maxY + textContainerOrigin.y + 4,
                width: max(80, bounds.width - textContainerOrigin.x * 2), height: view.cardHeight)
            bottom = max(bottom, view.frame.maxY)
        }
        let height = max(enclosingScrollView?.contentSize.height ?? bounds.height, bottom + 20)
        if abs(frame.height - height) > 1 { setFrameSize(NSSize(width: frame.width, height: height)) }
    }

    func resetSuggestions() {
        removeWorkingCue()
        pendingSuggestion?.cancel()
        pendingSuggestion = nil
        preview?.close()
        preview = nil
        marker?.removeFromSuperview()
        marker = nil
        suggestion = nil
        modelProposal = nil
        dismissedText = nil
    }

    func showWorking(_ active: Bool, document: String, at location: Int) {
        guard active, !previewsPaused, isEditable, !document.isEmpty, document == string else {
            removeWorkingCue()
            return
        }
        if workingDocument == document && workingLocation == location && workingToken != nil { return }
        removeWorkingCue()
        workingDocument = document
        workingLocation = location
        let token = UUID()
        workingToken = token
        let item = DispatchWorkItem { [weak self] in
            guard let self, self.workingToken == token, self.string == document,
                  !self.previewsPaused, self.window != nil else { return }
            let cue = NotesWorkingCue()
            self.workingCue = cue
            self.addSubview(cue)
            self.positionWorkingCue()
        }
        pendingWorkingCue = item
        // Avoid blinking a busy state for very fast or cached responses.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18, execute: item)
    }

    private func removeWorkingCue() {
        pendingWorkingCue?.cancel()
        pendingWorkingCue = nil
        workingToken = nil
        workingDocument = ""
        workingCue?.removeFromSuperview()
        workingCue = nil
    }

    private func positionWorkingCue() {
        guard let cue = workingCue, workingDocument == string,
              let manager = layoutManager, let container = textContainer else { return }
        let text = string as NSString
        guard text.length > 0 else { return }
        var paragraph = text.lineRange(for: NSRange(location: min(max(0, workingLocation), text.length - 1), length: 0))
        while paragraph.location > 0 && text.substring(with: paragraph).trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            paragraph = text.lineRange(for: NSRange(location: paragraph.location - 1, length: 0))
        }
        manager.ensureLayout(for: container)
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: paragraph.location, length: 1), actualCharacterRange: nil)
        let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
        cue.frame.origin = NSPoint(x: 0, y: rect.minY + textContainerOrigin.y)
    }

    func queueSuggestion() {
        if modelDriven { resetSuggestions(); return }
        pendingSuggestion?.cancel()
        preview?.close()
        marker?.isHidden = true
        guard !previewsPaused, isEditable, string != dismissedText else { return }
        let snapshot = string, identity = documentID
        let work = DispatchWorkItem { [weak self] in
            guard let self, !self.previewsPaused, self.string == snapshot,
                  self.documentID == identity, self.window != nil else { return }
            self.showSuggestion(in: snapshot)
        }
        pendingSuggestion = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2, execute: work)
    }

    private func showSuggestion(in text: String) {
        guard let found = NotesTableSuggestion.detect(in: text), !intersectsTable(found.sourceRange) else { return }
        suggestion = found
        suggestionSnapshot = text
        if marker == nil {
            let button = NSButton(image: NSImage(systemSymbolName: "tablecells", accessibilityDescription: "Preview table")!,
                                  target: self, action: #selector(openPreview))
            button.isBordered = false
            button.contentTintColor = .secondaryLabelColor
            button.toolTip = "Preview table formatting"
            button.setAccessibilityIdentifier("ems-table-cue")
            button.setAccessibilityLabel("Preview table formatting")
            marker = button
            addSubview(button)
        }
        marker?.isHidden = false
        positionMarker()
    }

    private func positionMarker() {
        let anchor = modelProposal?.range(in: string) ?? suggestion.flatMap { $0.stillMatches(string) ? $0.sourceRange : nil }
        guard let marker, !marker.isHidden, let anchor,
              let manager = layoutManager, let container = textContainer else { return }
        manager.ensureLayout(for: container)
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: anchor.location, length: 1), actualCharacterRange: nil)
        let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
        marker.frame = NSRect(x: 0, y: rect.minY + textContainerOrigin.y, width: 18, height: 20)
    }

    @objc private func openPreview() {
        if let modelProposal { openModelPreview(modelProposal); return }
        guard let suggestion, let marker, suggestionSnapshot == string else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.animates = false
        popover.contentViewController = NSHostingController(rootView: NotesTablePreview(suggestion: suggestion,
            insert: { [weak self] in self?.applySuggestion(suggestion) },
            dismiss: { [weak self] in
                guard let self else { return }
                self.dismissedText = self.string
                self.preview?.close()
                self.marker?.isHidden = true
            }))
        preview = popover
        popover.show(relativeTo: marker.bounds, of: marker, preferredEdge: .maxX)
    }

    func receive(_ proposals: [NotesProposal], document: String) {
        guard modelDriven, !previewsPaused, document == string,
              let proposed = proposals.first(where: { proposal in
                  guard let range = proposal.range(in: string) else { return false }
                  return !intersectsTable(range)
              }) else {
            if modelProposal != nil { resetSuggestions() }
            return
        }
        guard modelProposal != proposed || suggestionSnapshot != string else { return }
        resetSuggestions()
        modelProposal = proposed
        suggestionSnapshot = string
        let symbol = proposed.kind == "table" ? "tablecells" : proposed.kind == "priorities" ? "list.number" : proposed.kind == "structure" ? "textformat" : "text.alignleft"
        let button = NSButton(image: NSImage(systemSymbolName: symbol, accessibilityDescription: proposed.title)!, target: self, action: #selector(openPreview))
        button.isBordered = false
        button.contentTintColor = .secondaryLabelColor
        button.toolTip = proposed.title
        button.setAccessibilityIdentifier("ems-model-proposal")
        button.setAccessibilityLabel(proposed.title)
        marker = button
        addSubview(button)
        positionMarker()
    }

    private func openModelPreview(_ proposal: NotesProposal) {
        guard let marker, suggestionSnapshot == string else { return }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.animates = false
        let dismiss = { [weak self] in
            guard let self else { return }
            self.preview?.close()
            self.onProposalDismiss?(proposal)
            self.resetSuggestions()
        }
        if let table = proposal.table(in: string) {
            popover.contentViewController = NSHostingController(rootView: NotesTablePreview(suggestion: table,
                insert: { [weak self] in self?.applyModelProposal(proposal) }, dismiss: dismiss, attribution: modelLabel + " · suggestion"))
        } else {
            popover.contentViewController = NSHostingController(rootView: NotesProposalPreview(proposal: proposal,
                apply: { [weak self] in self?.applyModelProposal(proposal) }, dismiss: dismiss))
        }
        preview = popover
        popover.show(relativeTo: marker.bounds, of: marker, preferredEdge: .maxX)
    }

    func applyModelProposal(_ proposal: NotesProposal) {
        guard isEditable, !previewsPaused, suggestionSnapshot == string,
              let range = proposal.range(in: string), !intersectsTable(range) else { return }
        onProposalDismiss?(proposal)
        if let table = proposal.table(in: string) { applySuggestion(table); return }
        guard let body = proposal.text else { return }
        let target = proposal.kind == "breakdown" ? NSRange(location: NSMaxRange(range), length: 0) : range
        let value = proposal.kind == "breakdown" ? "\n\n" + body + "\n" : body
        replaceDocumentRange(target, with: NSAttributedString(string: value, attributes: MarkdownNoteStyle.base(size: 15)), action: "Apply " + proposal.title)
    }

    func applySuggestion(_ suggestion: NotesTableSuggestion) {
        guard !previewsPaused, isEditable, suggestion.stillMatches(string), !intersectsTable(suggestion.sourceRange) else { return }
        replaceDocumentRange(suggestion.sourceRange, with: NotesNativeTable.attributed(suggestion), action: "Insert table")
    }

    func replaceDocumentRange(_ range: NSRange, with replacement: NSAttributedString, action: String) {
        let selection = selectedRange()
        let scrollOrigin = enclosingScrollView?.contentView.bounds.origin
        preview?.close()
        insertText(replacement, replacementRange: range)
        undoManager?.setActionName(action)
        let delta = replacement.length - range.length
        let newStart: Int
        if selection.location < range.location { newStart = selection.location }
        else if selection.location >= NSMaxRange(range) { newStart = selection.location + delta }
        else { newStart = range.location + replacement.length }
        setSelectedRange(NSRange(location: min(max(0, newStart), (string as NSString).length), length: 0))
        if let scrollOrigin, let scroll = enclosingScrollView {
            scroll.contentView.scroll(to: scrollOrigin)
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        // Background document edits never take focus from another note or app.
        marker?.isHidden = true
    }

    func intersectsTable(_ range: NSRange) -> Bool {
        guard let storage = textStorage, NSMaxRange(range) <= storage.length else { return false }
        var found = false
        storage.enumerateAttribute(.paragraphStyle, in: range) { value, _, _ in
            if let paragraph = value as? NSParagraphStyle, !paragraph.textBlocks.isEmpty { found = true }
        }
        return found
    }

    func richDocument() throws -> Data? {
        guard let storage = textStorage,
              intersectsTable(NSRange(location: 0, length: storage.length)) || !imageReferences().isEmpty || !modelRanges().isEmpty else { return nil }
        let clean = NSMutableAttributedString(attributedString: storage)
        storage.enumerateAttribute(Self.originalParagraph, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            if let style = value as? NSParagraphStyle { clean.addAttribute(.paragraphStyle, value: style, range: range) }
        }
        clean.removeAttribute(.attachment, range: NSRange(location: 0, length: clean.length))
        clean.removeAttribute(.notesUserEdit, range: NSRange(location: 0, length: clean.length))
        clean.removeAttribute(.notesImageID, range: NSRange(location: 0, length: clean.length))
        clean.removeAttribute(.notesModelBlock, range: NSRange(location: 0, length: clean.length))
        clean.removeAttribute(Self.originalParagraph, range: NSRange(location: 0, length: clean.length))
        return try clean.data(from: NSRange(location: 0, length: clean.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
    }

    func focusIfReady() {
        guard focusOnAttach, window != nil else { return }
        focusOnAttach = false
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window else { return }
            window.makeFirstResponder(self)
        }
    }

    func format() {
        guard let storage = textStorage else { return }
        let selection = selectedRanges
        var preserved: [(NSAttributedString.Key, Any, NSRange)] = []
        for key in [NSAttributedString.Key.attachment, .notesImageID, .notesModelBlock, .notesUserEdit, .link] {
            storage.enumerateAttribute(key, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
                if let value {
                    if key == .link {
                        let url = (value as? URL) ?? (value as? String).flatMap(URL.init(string:))
                        guard url?.isFileURL == true, url?.pathExtension == "ics" else { return }
                    }
                    preserved.append((key, value, range))
                }
            }
        }
        var tables: [(NSRange, NSParagraphStyle)] = []
        storage.enumerateAttribute(.paragraphStyle, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            if let paragraph = value as? NSParagraphStyle, !paragraph.textBlocks.isEmpty {
                tables.append((range, paragraph))
            }
        }
        let recording = undoManager?.isUndoRegistrationEnabled == true
        if recording { undoManager?.disableUndoRegistration() }
        MarkdownNoteStyle.apply(to: storage, size: 15)
        for (range, paragraph) in tables {
            storage.setAttributes([.paragraphStyle: paragraph, .font: NSFont.systemFont(ofSize: 15), .foregroundColor: NSColor.labelColor], range: range)
            for block in paragraph.textBlocks { block.setBorderColor(.separatorColor) }
        }
        for (key, value, range) in preserved { storage.addAttribute(key, value: value, range: range) }
        NotesTypography.apply(to: storage, authored: NotesPassage.passages(in: string, excluding: modelRanges().map(\.range)))
        styleChecklistAndUserEdits()
        decorateInline()
        if recording { undoManager?.enableUndoRegistration() }
        selectedRanges = selection
        var typing = MarkdownNoteStyle.base(size: 16)
        typing[.font] = ShellTypography.nativeSerif(16)
        if storage.length > 0 {
            let index = min(selectedRange().location, storage.length - 1)
            if let paragraph = storage.attribute(.paragraphStyle, at: index, effectiveRange: nil) as? NSParagraphStyle,
               !paragraph.textBlocks.isEmpty { typing[.paragraphStyle] = paragraph }
        }
        typingAttributes = typing
        insertionPointColor = .controlAccentColor
        formattedText = string
    }
}
