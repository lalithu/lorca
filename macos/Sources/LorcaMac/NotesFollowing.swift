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
    var event: NotesCalendarEvent? = nil
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

/// Blank lines delimit passages; content and intent are left to the model.
/// Exact source identity lets a passage survive edits elsewhere in the note.
struct NotesPassage: Equatable {
    let source: String
    let range: NSRange
    static func passages(in text: String, excluding: [NSRange] = []) -> [NotesPassage] {
        let masked = NSMutableString(string: text)
        for range in excluding where range.location >= 0 && NSMaxRange(range) <= masked.length {
            masked.replaceCharacters(in: range, with: String(repeating: " ", count: range.length))
        }
        let text = masked as String
        let value = text as NSString
        let separator = try! NSRegularExpression(pattern: #"\r?\n[ \t]*\r?\n+|\r?\n(?=#{1,6}[ \t]+|[^\s#*\-\d][^\r\n]{2,100}:[ \t]*(?:\r?\n|$))"#)
        let cuts = separator.matches(in: text, range: NSRange(location: 0, length: value.length))
        var start = 0
        var ranges: [NSRange] = []
        for cut in cuts { ranges.append(NSRange(location: start, length: cut.range.location - start)); start = NSMaxRange(cut.range) }
        ranges.append(NSRange(location: start, length: value.length - start))
        var grouped: [NSRange] = []
        let list = try! NSRegularExpression(pattern: #"^\s*(?:[-*+] |\d+[.)] )"#)
        for range in ranges {
            let raw = value.substring(with: range)
            if let previous = grouped.last, list.firstMatch(in: raw, range: NSRange(location: 0, length: (raw as NSString).length)) != nil {
                grouped[grouped.count - 1] = NSRange(location: previous.location, length: NSMaxRange(range) - previous.location)
            } else if !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { grouped.append(range) }
        }
        let passages = grouped.compactMap { range -> NotesPassage? in
            let raw = value.substring(with: range)
            let source = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            guard (source.count >= 12 || source.contains("\u{fffc}")), source.utf8.count <= 12_000 else { return nil }
            let local = (raw as NSString).range(of: source)
            return NotesPassage(source: source, range: NSRange(location: range.location + local.location, length: local.length))
        }
        // Ambiguous duplicate anchors must never receive an edit or decoration.
        return passages.filter { passage in passages.filter { $0.source == passage.source }.count == 1 }
    }
}

struct NotesSectionResult: Codable, Equatable {
    static let currentVersion = 3
    var version: Int? = nil
    let source: String
    let model: String
    var proposals: [NotesProposal]
    var unread: Bool
}

struct NotesInlineState: Equatable {
    let source: String
    let label: String
    var unread = false
    var proposal: NotesProposal? = nil
    var busy = false
    var detail: String? = nil
}

@MainActor
final class NotesFollowing: ObservableObject {
    @Published private(set) var proposals: [NotesProposal] = []
    @Published private(set) var status = "Waiting for writing"
    @Published private(set) var working = false
    @Published private(set) var model = ""
    @Published private(set) var partialContext = false
    @Published private(set) var inline: [NotesInlineState] = []
    private(set) var proposalDocument = ""
    private(set) var workingDocument = ""
    private(set) var workingLocation = 0
    private(set) var visible = false
    private let notes: NotesStore
    private let chat: ChatStore
    private var observations = Set<AnyCancellable>()
    private var pending: Task<Void, Never>?
    private var wake: Task<Void, Never>?
    private var revision: UInt64 = 0
    private var requestID: UUID?
    private var active: (note: UUID, source: String, model: String, route: String)?
    private var activated = Set<UUID>()
    private var changed: [String: Date] = [:]
    private var failures: [String: String] = [:]
    private var activity = "Thinking…"
    private let delay: TimeInterval

    init(notes: NotesStore, chat: ChatStore, delayNanoseconds: UInt64 = 1_500_000_000) {
        self.notes = notes; self.chat = chat
        self.delay = Double(delayNanoseconds) / 1_000_000_000
        for changes in [notes.objectWillChange.eraseToAnyPublisher(), chat.objectWillChange.eraseToAnyPublisher()] {
            changes.sink { [weak self] _ in
                Task { @MainActor [weak self] in await Task.yield(); self?.reconsider() }
            }.store(in: &observations)
        }
    }

    func setVisible(_ value: Bool) {
        visible = value
        if value { activated.insert(notes.selectedID) }
        reconsider()
    }
    func refresh() {
        failures = [:]
        notes.setFollowingResults([], for: notes.selectedID)
        reconsider()
    }
    func retry(source: String) {
        failures.removeValue(forKey: key(notes.selectedID, source))
        let kept = (notes.current.followingResults ?? []).filter { $0.source != source }
        notes.setFollowingResults(kept, for: notes.selectedID)
        reconsider()
    }
    func dismiss(_ proposal: NotesProposal) {
        var results = notes.current.followingResults ?? []
        for index in results.indices { results[index].proposals.removeAll { $0.id == proposal.id } }
        notes.setFollowingResults(results, for: notes.selectedID)
        publish()
    }
    func markRead(source: String) {
        var results = notes.current.followingResults ?? []
        for index in results.indices where results[index].source == source { results[index].unread = false }
        notes.setFollowingResults(results, for: notes.selectedID)
    }
    private func key(_ note: UUID, _ source: String) -> String { note.uuidString + source }
    private var route: String { chat.modelChoice.map { $0.route.provider + $0.route.host } ?? "" }
    private var busy: Bool { chat.isGenerating || chat.agent.busy || chat.jobs.running || chat.isManagingModel || chat.isSelectingModel }

    private func cancel() {
        pending?.cancel(); pending = nil
        if let id = requestID {
            let backend = chat.backend
            Task { _ = try? await backend.command(["op":"notes_cancel", "requestId":id.uuidString]) }
        }
        requestID = nil; active = nil; working = false
    }

    private func reconsider() {
        wake?.cancel(); wake = nil
        if visible { activated.insert(notes.selectedID) }
        model = chat.selectedModel
        let liveKeys = Set(notes.notes.filter { activated.contains($0.id) }.flatMap { note in
            note.passages.map { key(note.id, $0.source) }
        })
        changed = changed.filter { liveKeys.contains($0.key) }
        failures = failures.filter { liveKeys.contains($0.key) }
        if let active, notes.previewsPaused || busy || !chat.isConnected || model != active.model || route != active.route ||
            !(notes.notes.first { $0.id == active.note }.map { $0.passages.contains { $0.source == active.source } } ?? false) {
            cancel()
        }
        publish()
        guard !notes.previewsPaused, !notes.isReadOnly, chat.ready, chat.isConnected, !busy else { return }
        guard active == nil else { return }
        var wait: TimeInterval?
        for note in notes.notes where activated.contains(note.id) {
            let results = note.followingResults ?? []
            for passage in note.passages {
                let identity = key(note.id, passage.source)
                guard !results.contains(where: { $0.source == passage.source && $0.model == model && $0.version == NotesSectionResult.currentVersion }), failures[identity] == nil else { continue }
                let firstSeen = changed[identity] ?? Date()
                changed[identity] = firstSeen
                let caretHere = visible && note.id == notes.selectedID && (passage.range.location...NSMaxRange(passage.range)).contains(note.selection.location)
                let remaining = caretHere ? delay - Date().timeIntervalSince(firstSeen) : 0
                if remaining > 0 { wait = min(wait ?? remaining, remaining); continue }
                start(note: note, passage: passage)
                return
            }
        }
        if let wait {
            wake = Task { [weak self] in
                do { try await Task.sleep(nanoseconds: UInt64(max(0.001, wait) * 1_000_000_000)); self?.reconsider() } catch { }
            }
        }
    }

    private func publish() {
        let note = notes.current
        proposalDocument = note.text
        let valid = Set(note.passages.map(\.source))
        let results = (note.followingResults ?? []).filter { valid.contains($0.source) && $0.model == model && $0.version == NotesSectionResult.currentVersion }
        proposals = results.flatMap(\.proposals).filter { valid.contains($0.source) }
        inline = []
        for passage in note.passages {
            if let error = failures[key(note.id, passage.source)] {
                inline.append(NotesInlineState(source: passage.source, label: "Couldn’t finish · click to retry", detail: error))
                status = error
            }
        }
        working = active != nil
        workingDocument = ""; workingLocation = 0
        if let active, active.note == note.id, let passage = note.passages.first(where: { $0.source == active.source }) {
            workingDocument = note.text; workingLocation = passage.range.location
            inline.append(NotesInlineState(source: active.source, label: activity, busy: true))
        }
        if notes.previewsPaused { status = "Paused" }
        else if !chat.ready || !chat.isConnected { status = "Connect a local model in Settings" }
        else if busy { status = "Waiting for foreground work" }
        else if working { status = activity }
        else if !proposals.isEmpty { status = "Following" }
        else if failures.isEmpty { status = "Following" }
    }

    private func start(note: WorkspaceNote, passage: NotesPassage) {
        revision &+= 1
        let rev = revision, id = UUID(), selectedModel = model
        requestID = id
        active = (note.id, passage.source, selectedModel, route)
        activity = "Thinking…"
        publish()
        let backend = chat.backend
        pending = Task { [weak self] in
            do {
                guard let self else { return }
                let clock = ISO8601DateFormatter(); clock.timeZone = .current
                var fields: [String: Any] = ["op":"notes_follow", "requestId":id.uuidString,
                    "noteId":note.id.uuidString, "revision":rev, "text":passage.source,
                    "localTime":clock.string(from: Date()) + " " + TimeZone.current.identifier]
                let images = (note.images ?? []).filter { NSLocationInRange($0.location, passage.range) }
                if !images.isEmpty {
                    self.activity = "Reading image…"; self.publish()
                    let directory = self.notes.mediaDirectory
                    let reading = try await Task.detached(priority: .userInitiated) {
                        try NotesImageReading.read(images, directory: directory, passage: passage.source)
                    }.value
                    try Task.checkCancellation()
                    guard self.requestID == id else { return }
                    fields["images"] = reading.images
                    fields["imageObservations"] = reading.observations
                    fields["dateCandidates"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(reading.dates))
                    self.activity = "Captioning…"; self.publish()
                }
                var data = try await backend.command(fields)
                try Task.checkCancellation()
                var result = try JSONDecoder().decode(NotesFollowingResult.self, from: data)
                guard result.noteId == note.id, result.revision == rev, result.requestId == id, result.model == selectedModel else {
                    throw BackendError(message: "Model result did not match this passage. Retry this passage.")
                }
                if let research = result.suggestions.first(where: { $0.kind == "research" }), let query = research.text {
                    guard self.requestID == id else { return }
                    // Public task label, never hidden reasoning. Search now really starts.
                    self.activity = research.title
                    self.publish()
                    fields["researchQuery"] = query
                    data = try await backend.command(fields)
                    try Task.checkCancellation()
                    result = try JSONDecoder().decode(NotesFollowingResult.self, from: data)
                    guard result.noteId == note.id, result.revision == rev, result.requestId == id, result.model == selectedModel else {
                        throw BackendError(message: "Research result did not match this passage. Retry this passage.")
                    }
                }
                guard self.requestID == id, !self.notes.previewsPaused, self.model == selectedModel,
                      result.noteId == note.id, result.revision == rev, result.requestId == id, result.model == selectedModel,
                      let current = self.notes.notes.first(where: { $0.id == note.id }),
                      current.passages.contains(where: { $0.source == passage.source }) else { return }
                let proposals = result.suggestions.filter { proposal in
                    proposal.kind != "research" && current.passages.contains(where: { $0.source == proposal.source })
                }
                var saved = current.followingResults ?? []
                saved.removeAll { $0.source == passage.source }
                saved.append(NotesSectionResult(version: NotesSectionResult.currentVersion, source: passage.source, model: selectedModel, proposals: proposals, unread: true))
                self.notes.setFollowingResults(saved, for: note.id)
                let document = self.notes.document(for: note.id)
                for proposal in proposals { try document.add(proposal) }
                self.active = nil; self.requestID = nil; self.pending = nil
                self.reconsider()
            } catch is CancellationError { }
            catch {
                guard let self, self.requestID == id else { return }
                self.failures[self.key(note.id, passage.source)] = error.localizedDescription
                self.active = nil; self.requestID = nil; self.pending = nil
                self.reconsider()
            }
        }
    }

    static func context(for note: WorkspaceNote) -> String {
        guard note.text.utf8.count > 12_000 else { return note.text }
        let chars = Array(note.text)
        let prefix = (note.text as NSString).substring(to: note.selection.location)
        let cursor = min(prefix.count, chars.count)
        let start = max(0, cursor - 2_000)
        return String(chars[start..<min(chars.count, start + 4_000)])
    }
}
