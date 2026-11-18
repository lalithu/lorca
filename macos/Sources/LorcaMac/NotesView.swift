import AppKit
import SwiftUI

struct NotesView: View {
    @ObservedObject var store: NotesStore

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

            NotesTextEditor(store: store)
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if let error = store.error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled).padding(.vertical, 8)
            }
            HStack(spacing: 6) {
                Menu {
                    Button(store.previewsPaused ? "Resume suggestions" : "Pause suggestions") {
                        store.previewsPaused.toggle()
                    }
                    Text("UI preview — no model connected")
                } label: {
                    Label(store.previewsPaused ? "Ems paused" : "Ems preview", systemImage: "sparkles")
                }
                .menuStyle(.borderlessButton).fixedSize()
                .help("Local UI preview. No model is observing your note.")
                Spacer()
                Text("\(store.wordCount) \(store.wordCount == 1 ? "word" : "words")")
            }
            .font(.system(size: 10)).foregroundStyle(.secondary)
            .padding(.top, 12).padding(.bottom, 18)
        }
        .padding(.horizontal, 32)
    }
}

struct NotesHistory: View {
    @ObservedObject var store: NotesStore

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(store.notes) { note in
                Button { store.select(note.id) } label: {
                    Text(note.title)
                        .font(.system(size: 11)).lineLimit(1)
                        .foregroundStyle(note.id == store.selectedID ? .primary : .secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.leading, 30).padding(.trailing, 8).padding(.vertical, 7)
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

    func makeCoordinator() -> Coordinator { Coordinator(store: store) }

    func makeNSView(context: Context) -> NSScrollView {
        let editor = NotesNativeTextView()
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
        editor.delegate = context.coordinator
        editor.setAccessibilityIdentifier("workspace-notes-editor")
        editor.setAccessibilityLabel("Note document")
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = editor
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let editor = scroll.documentView as? NotesNativeTextView else { return }
        let coordinator = context.coordinator
        coordinator.store = store
        coordinator.applying = true
        defer { coordinator.applying = false }
        let note = store.current
        let changedNote = coordinator.displayedID != note.id
        let changedFocus = coordinator.focusRequest != store.focusRequest
        if changedNote {
            editor.undoManager?.removeAllActions()
            coordinator.displayedID = note.id
            editor.resetSuggestions()
            editor.documentID = note.id
        }
        if changedNote || editor.string != note.text {
            if let data = note.richText,
               let rich = try? NSAttributedString(data: data, options: [.documentType: NSAttributedString.DocumentType.rtf], documentAttributes: nil),
               rich.string == note.text {
                editor.textStorage?.setAttributedString(rich)
            } else { editor.string = note.text }
        }
        if editor.previewsPaused != store.previewsPaused {
            editor.previewsPaused = store.previewsPaused
            if store.previewsPaused { editor.resetSuggestions() }
            else { editor.queueSuggestion() }
        }
        editor.isEditable = !store.isReadOnly
        let selection = changedNote ? note.selection : editor.selectedRange()
        if changedNote || editor.formattedText != note.text {
            editor.format()
            editor.setSelectedRange(selection)
        }
        if changedNote || changedFocus {
            coordinator.focusRequest = store.focusRequest
            editor.setSelectedRange(note.selection)
            editor.scrollRangeToVisible(note.selection)
            editor.focusOnAttach = !store.isReadOnly
            editor.focusIfReady()
        }
        if changedNote { editor.queueSuggestion() }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var store: NotesStore
        var displayedID: UUID?
        var focusRequest: UUID?
        var applying = false
        init(store: NotesStore) { self.store = store }

        func textDidChange(_ notification: Notification) {
            guard !applying, let editor = notification.object as? NotesNativeTextView,
                  displayedID == store.selectedID else { return }
            let text = editor.string, selection = editor.selectedRange()
            editor.format()
            do {
                store.updateText(text, selection: selection, richText: try editor.richDocument())
            } catch {
                store.reportEditorError("This note could not be saved. Keep it open. \(error.localizedDescription)")
            }
            editor.queueSuggestion()
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard !applying, let editor = notification.object as? NotesNativeTextView,
                  displayedID == store.selectedID, editor.string == store.current.text else { return }
            store.updateSelection(editor.selectedRange())
        }
    }
}

final class NotesNativeTextView: NSTextView {
    var formattedText: String?
    var focusOnAttach = false
    var documentID: UUID?
    var previewsPaused = false
    private var pendingSuggestion: DispatchWorkItem?
    private var suggestion: NotesTableSuggestion?
    private var suggestionSnapshot = ""
    private var dismissedText: String?
    private var marker: NSButton?
    private var preview: NSPopover?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        focusIfReady()
        if window == nil { resetSuggestions() }
    }

    override func layout() {
        super.layout()
        positionMarker()
    }

    func resetSuggestions() {
        pendingSuggestion?.cancel()
        pendingSuggestion = nil
        preview?.close()
        preview = nil
        marker?.removeFromSuperview()
        marker = nil
        suggestion = nil
        dismissedText = nil
    }

    func queueSuggestion() {
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
        guard let marker, !marker.isHidden, let suggestion,
              suggestion.stillMatches(string), let manager = layoutManager, let container = textContainer else { return }
        manager.ensureLayout(for: container)
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: suggestion.sourceRange.location, length: 1), actualCharacterRange: nil)
        let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
        marker.frame = NSRect(x: 0, y: rect.minY + textContainerOrigin.y, width: 18, height: 20)
    }

    @objc private func openPreview() {
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

    func applySuggestion(_ suggestion: NotesTableSuggestion) {
        guard !previewsPaused, isEditable, suggestion.stillMatches(string), !intersectsTable(suggestion.sourceRange) else { return }
        let table = NotesNativeTable.attributed(suggestion)
        let selection = selectedRange()
        let scrollOrigin = enclosingScrollView?.contentView.bounds.origin
        preview?.close()
        insertText(table, replacementRange: suggestion.sourceRange)
        undoManager?.setActionName("Insert table")
        let delta = table.length - suggestion.sourceRange.length
        let newStart: Int
        if selection.location < suggestion.sourceRange.location { newStart = selection.location }
        else if selection.location >= NSMaxRange(suggestion.sourceRange) { newStart = selection.location + delta }
        else { newStart = suggestion.sourceRange.location + table.length }
        setSelectedRange(NSRange(location: min(max(0, newStart), (string as NSString).length), length: 0))
        if let scrollOrigin, let scroll = enclosingScrollView {
            scroll.contentView.scroll(to: scrollOrigin)
            scroll.reflectScrolledClipView(scroll.contentView)
        }
        window?.makeFirstResponder(self)
        marker?.isHidden = true
    }

    private func intersectsTable(_ range: NSRange) -> Bool {
        guard let storage = textStorage, NSMaxRange(range) <= storage.length else { return false }
        var found = false
        storage.enumerateAttribute(.paragraphStyle, in: range) { value, _, _ in
            if let paragraph = value as? NSParagraphStyle, !paragraph.textBlocks.isEmpty { found = true }
        }
        return found
    }

    func richDocument() throws -> Data? {
        guard let storage = textStorage, intersectsTable(NSRange(location: 0, length: storage.length)) else { return nil }
        return try storage.data(from: NSRange(location: 0, length: storage.length), documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf])
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
        if recording { undoManager?.enableUndoRegistration() }
        selectedRanges = selection
        var typing = MarkdownNoteStyle.base(size: 15)
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
