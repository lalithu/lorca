import Foundation
import Combine

struct WorkspaceNote: Codable, Identifiable, Equatable {
    var id = UUID()
    var text = ""
    var createdAt = Date()
    var updatedAt = Date()
    var selectionLocation = 0
    var selectionLength = 0

    var title: String {
        text.split(whereSeparator: \.isNewline).first.map { String($0.prefix(60)) } ?? "New note"
    }

    var selection: NSRange {
        let length = (text as NSString).length
        let start = min(max(0, selectionLocation), length)
        return NSRange(location: start, length: min(max(0, selectionLength), length - start))
    }
}

@MainActor
final class NotesStore: ObservableObject {
    private struct Document: Codable {
        var version = 1
        var selectedID: UUID
        var notes: [WorkspaceNote]
    }

    @Published private(set) var notes: [WorkspaceNote]
    @Published private(set) var selectedID: UUID
    @Published private(set) var focusRequest = UUID()
    @Published private(set) var error: String?
    private(set) var isReadOnly = false
    let file: URL

    init(root: URL) {
        let initial = WorkspaceNote()
        notes = [initial]
        selectedID = initial.id
        file = root.appendingPathComponent("notes/workspace.json")
        guard FileManager.default.fileExists(atPath: file.path) else { return }
        do {
            let data = try JSONDecoder().decode(Document.self, from: Data(contentsOf: file))
            guard data.version == 1, !data.notes.isEmpty,
                  Set(data.notes.map(\.id)).count == data.notes.count else {
                throw CocoaError(.fileReadCorruptFile)
            }
            notes = data.notes
            selectedID = notes.contains(where: { $0.id == data.selectedID }) ? data.selectedID : notes[0].id
        } catch {
            isReadOnly = true
            self.error = "Notes could not be opened. The saved file was left untouched. \(error.localizedDescription)"
        }
    }

    var current: WorkspaceNote { notes.first(where: { $0.id == selectedID }) ?? notes[0] }
    var wordCount: Int { current.text.split(whereSeparator: \.isWhitespace).count }

    func create() {
        guard !isReadOnly else { return }
        let note = WorkspaceNote()
        notes.insert(note, at: 0)
        selectedID = note.id
        focusRequest = UUID()
        save()
    }

    func select(_ id: UUID) {
        guard notes.contains(where: { $0.id == id }) else { return }
        selectedID = id
        focusRequest = UUID()
        save()
    }

    func updateText(_ text: String, selection: NSRange) {
        guard !isReadOnly, let index = notes.firstIndex(where: { $0.id == selectedID }) else { return }
        guard notes[index].text != text else { return }
        notes[index].text = text
        notes[index].updatedAt = Date()
        notes[index].selectionLocation = selection.location
        notes[index].selectionLength = selection.length
        save()
    }

    func updateSelection(_ selection: NSRange) {
        guard !isReadOnly, let index = notes.firstIndex(where: { $0.id == selectedID }) else { return }
        guard notes[index].selection != selection else { return }
        notes[index].selectionLocation = selection.location
        notes[index].selectionLength = selection.length
        save()
    }

    private func save() {
        guard !isReadOnly else { return }
        do {
            try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            let data = try JSONEncoder().encode(Document(selectedID: selectedID, notes: notes))
            try data.write(to: file, options: .atomic)
            error = nil
        } catch {
            self.error = "Notes could not be saved. Keep this window open. \(error.localizedDescription)"
        }
    }
}
