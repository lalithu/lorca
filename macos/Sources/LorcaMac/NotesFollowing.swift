import AppKit
import Combine
import Foundation

struct NotesProposal: Codable, Equatable, Identifiable {
    let kind: String
    let source: String
    let title: String
    let text: String?
    let headers: [String]?
    let rows: [[String]]?
    var id: String { kind + ":" + source }

    func range(in document: String) -> NSRange? {
        guard !source.isEmpty else { return nil }
        let value = document as NSString
        let range = value.range(of: source)
        guard range.location != NSNotFound else { return nil }
        let rest = NSRange(location: NSMaxRange(range), length: value.length - NSMaxRange(range))
        guard value.range(of: source, range: rest).location == NSNotFound else { return nil }
        return range
    }

    func table(in document: String) -> NotesTableSuggestion? {
        guard kind == "table", let range = range(in: document), let headers, let rows,
              !headers.isEmpty, headers.count <= 6, !rows.isEmpty,
              rows.allSatisfy({ $0.count == headers.count }) else { return nil }
        return NotesTableSuggestion(sourceRange: range, sourceText: source, headers: headers, rows: rows)
    }
}

struct NotesFollowingResult: Decodable {
    let noteId: UUID
    let revision: UInt64
    let requestId: UUID
    let model: String
    let suggestions: [NotesProposal]
}

@MainActor
final class NotesFollowing: ObservableObject {
    @Published private(set) var proposals: [NotesProposal] = []
    @Published private(set) var status = "Waiting for writing"
    @Published private(set) var working = false
    @Published private(set) var model = ""
    @Published private(set) var partialContext = false
    private(set) var proposalDocument = ""
    private(set) var workingDocument = ""
    private(set) var workingLocation = 0
    private var lastNoteID: UUID?
    private var lastText = ""
    private(set) var visible = false
    private let notes: NotesStore
    private let chat: ChatStore
    private var observations = Set<AnyCancellable>()
    private var pending: Task<Void, Never>?
    private var revision: UInt64 = 0
    private var requestID: UUID?
    private var lastKey = ""
    private var dismissed = Set<String>()
    private let delay: UInt64

    init(notes: NotesStore, chat: ChatStore, delayNanoseconds: UInt64 = 1_500_000_000) {
        self.notes = notes
        self.chat = chat
        self.delay = delayNanoseconds
        for changes in [notes.objectWillChange.eraseToAnyPublisher(), chat.objectWillChange.eraseToAnyPublisher()] {
            changes.sink { [weak self] _ in
                // Combine publishes before mutation; read a coherent snapshot on
                // the next main-actor turn, without doing inference on that actor.
                Task { @MainActor [weak self] in await Task.yield(); self?.reconsider() }
            }.store(in: &observations)
        }
    }

    func setVisible(_ value: Bool) { visible = value; reconsider() }
    func refresh() { lastKey = ""; reconsider() }
    func dismiss(_ proposal: NotesProposal) {
        dismissed.insert(proposal.id)
        proposals.removeAll { $0.id == proposal.id }
        status = proposals.isEmpty ? "Following" : "Suggestions available"
    }

    private func cancel() {
        pending?.cancel()
        pending = nil
        if let id = requestID {
            let backend = chat.backend
            Task { _ = try? await backend.command(["op":"notes_cancel", "requestId":id.uuidString]) }
        }
        requestID = nil
        workingDocument = ""
        working = false
    }

    private func reconsider() {
        let note = notes.current
        let route = chat.modelChoice.map { $0.route.provider + $0.route.host } ?? ""
        let busy = chat.isGenerating || chat.agent.busy || chat.jobs.running || chat.isManagingModel || chat.isSelectingModel
        let context = Self.context(for: note)
        let key = "\(visible)|\(notes.previewsPaused)|\(chat.ready)|\(chat.isConnected)|\(busy)|\(chat.selectedModel)|\(route)|\(note.id)|\(note.text)|\(context)"
        guard key != lastKey else { return }
        lastKey = key
        revision &+= 1
        cancel()
        proposals = []
        proposalDocument = ""
        if lastNoteID != note.id || lastText != note.text { dismissed = [] }
        lastNoteID = note.id; lastText = note.text
        model = chat.selectedModel
        partialContext = context != note.text
        guard visible else { status = "Paused outside Notes"; return }
        guard !notes.previewsPaused else { status = "Paused"; return }
        guard !notes.isReadOnly else { status = "Notes are read-only"; return }
        guard chat.ready && chat.isConnected else { status = "Connect a local model in Settings"; return }
        guard !busy else { status = "Waiting for foreground work"; return }
        guard context.trimmingCharacters(in: .whitespacesAndNewlines).count >= 12 else { status = "Waiting for writing"; return }
        status = "Waiting for a pause"
        let rev = revision, fullText = note.text, noteID = note.id, selectedModel = chat.selectedModel
        let backend = chat.backend
        pending = Task { [weak self] in
            do {
                guard let self else { return }
                try await Task.sleep(nanoseconds: self.delay)
                try Task.checkCancellation()
                guard self.revision == rev else { return }
                let id = UUID()
                self.requestID = id
                self.workingDocument = fullText
                self.workingLocation = note.selection.location
                self.working = true
                self.status = "Following"
                let clock = ISO8601DateFormatter()
                clock.timeZone = .current
                let localTime = clock.string(from: Date()) + " " + TimeZone.current.identifier
                let data = try await backend.command(["op":"notes_follow", "requestId":id.uuidString,
                    "noteId":noteID.uuidString, "revision":rev, "text":context, "localTime":localTime])
                try Task.checkCancellation()
                let result = try JSONDecoder().decode(NotesFollowingResult.self, from: data)
                guard self.revision == rev, self.visible, !self.notes.previewsPaused,
                      self.notes.current.id == noteID, self.notes.current.text == fullText,
                      self.chat.selectedModel == selectedModel,
                      result.noteId == noteID, result.revision == rev, result.requestId == id,
                      result.model == selectedModel else { return }
                self.proposalDocument = fullText
                self.proposals = result.suggestions.filter { $0.range(in: fullText) != nil && !self.dismissed.contains($0.id) }
                self.status = self.proposals.isEmpty ? "Following" : "Suggestions available"
                self.requestID = nil
                self.working = false
            } catch is CancellationError {
                // The newer revision already owns the UI state.
            } catch {
                guard let self, self.revision == rev else { return }
                self.requestID = nil
                self.working = false
                self.status = error.localizedDescription
                // No polling or automatic retry loop. A new edit, resume, model
                // selection, or explicit refresh is the next opportunity.
            }
        }
    }

    static func context(for note: WorkspaceNote) -> String {
        guard note.text.utf8.count > 12_000 else { return note.text }
        // Keep a bounded passage around the caret. Never split a UTF-16 pair.
        let chars = Array(note.text)
        let prefix = (note.text as NSString).substring(to: note.selection.location)
        let cursor = min(prefix.count, chars.count)
        let start = max(0, cursor - 2_000)
        return String(chars[start..<min(chars.count, start + 4_000)])
    }
}
