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
                Image(systemName: "sparkles")
                Text("Model following not connected")
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
        editor.isRichText = false
        editor.importsGraphics = false
        editor.allowsUndo = true
        editor.drawsBackground = false
        editor.isVerticallyResizable = true
        editor.isHorizontallyResizable = false
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        editor.textContainer?.lineFragmentPadding = 0
        editor.textContainerInset = NSSize(width: 0, height: 0)
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
        }
        if editor.string != note.text { editor.string = note.text }
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
            store.updateText(text, selection: selection)
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

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        focusIfReady()
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
        let recording = undoManager?.isUndoRegistrationEnabled == true
        if recording { undoManager?.disableUndoRegistration() }
        MarkdownNoteStyle.apply(to: storage, size: 15)
        if recording { undoManager?.enableUndoRegistration() }
        selectedRanges = selection
        typingAttributes = MarkdownNoteStyle.base(size: 15)
        insertionPointColor = .controlAccentColor
        formattedText = string
    }
}
