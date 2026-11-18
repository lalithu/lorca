import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class FollowingMockBackend: MockBackend {
    var followRequests: [[String: Any]] = []
    var cancellations: [String] = []
    override func command(_ fields: [String: Any]) async throws -> Data {
        if fields["op"] as? String == "notes_cancel" {
            cancellations.append(fields["requestId"] as? String ?? "")
            return Data("{}".utf8)
        }
        guard fields["op"] as? String == "notes_follow" else { return try await super.command(fields) }
        followRequests.append(fields)
        // Deliberately finish even after client cancellation: revision fencing
        // must protect the UI independently of transport cooperation.
        await withCheckedContinuation { continuation in
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { continuation.resume() }
        }
        return try JSONSerialization.data(withJSONObject: ["noteId":fields["noteId"]!, "revision":fields["revision"]!,
            "requestId":fields["requestId"]!, "model":model,
            "suggestions":[["kind":"breakdown","source":fields["text"]!,"title":"Next steps","text":"An outline"]]])
    }
}

@MainActor
final class NotesFollowingTests: XCTestCase {
    func testPauseSurvivesRestartAndStaleNativeProposalCannotApply() throws {
        _ = NSApplication.shared
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let notes = NotesStore(root: root)
        notes.previewsPaused = true
        XCTAssertTrue(NotesStore(root: root).previewsPaused)
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 500, height: 300))
        editor.modelDriven = true
        editor.string = "A tomorrow\nB tonight"
        let proposal = NotesProposal(kind: "priorities", source: editor.string, title: "Tonight first",
            text: "B tonight\nA tomorrow", headers: nil, rows: nil)
        editor.receive([proposal], document: editor.string)
        editor.string += "\nA new thought"
        let updated = editor.string
        editor.applyModelProposal(proposal)
        XCTAssertEqual(editor.string, updated)
        editor.receive([proposal], document: "old document")
        XCTAssertTrue(editor.subviews.compactMap { $0 as? NSButton }.isEmpty)
    }

    func testLatestRevisionWinsWithoutMutatingTheNote() async throws {
        let backend = FollowingMockBackend()
        let suite = "lorca.notes-following.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 5_000_000)
        following.setVisible(true)
        notes.updateText("First task needs a study guide", selection: NSRange(location: 0, length: 0))
        try await Task.sleep(for: .milliseconds(30))
        notes.updateText("Latest task needs a study guide", selection: NSRange(location: 0, length: 0))
        try await Task.sleep(for: .milliseconds(180))
        XCTAssertEqual(following.proposals.first?.source, notes.current.text)
        XCTAssertEqual(notes.current.text, "Latest task needs a study guide")
        XCTAssertFalse(backend.cancellations.isEmpty)
        XCTAssertEqual(backend.followRequests.count, 2)
        following.setVisible(false)
        XCTAssertTrue(following.proposals.isEmpty)
    }

    func testPausedOrHiddenNotesDoNotSendWorkAndRapidEditsCoalesce() async throws {
        let backend = FollowingMockBackend()
        let suite = "lorca.notes-debounce.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 25_000_000)
        notes.updateText("Need to organize these assignments", selection: NSRange(location: 0, length: 0))
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertTrue(backend.followRequests.isEmpty)
        notes.previewsPaused = true
        following.setVisible(true)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertTrue(backend.followRequests.isEmpty)
        notes.previewsPaused = false
        for i in 0..<5 { notes.updateText("Assignment list revision \(i)", selection: NSRange(location: 0, length: 0)) }
        try await Task.sleep(for: .milliseconds(180))
        XCTAssertEqual(backend.followRequests.count, 1)
        XCTAssertEqual(following.proposals.first?.source, "Assignment list revision 4")
        following.setVisible(false)
    }

    func testAnchorsRejectDuplicatePassagesAndLongContextKeepsUnicode() {
        let proposal = NotesProposal(kind: "breakdown", source: "same", title: "Outline", text: "body", headers: nil, rows: nil)
        XCTAssertNil(proposal.range(in: "same then same"))
        XCTAssertNotNil(proposal.range(in: "A unique same passage"))
        var note = WorkspaceNote()
        note.text = String(repeating: "🐬 a paragraph\n", count: 4000)
        note.selectionLocation = (note.text as NSString).length
        let context = NotesFollowing.context(for: note)
        XCTAssertLessThan(context.utf8.count, 24_000)
        XCTAssertFalse(context.contains("�"))
        XCTAssertTrue(note.text.hasSuffix(context))
    }
}
