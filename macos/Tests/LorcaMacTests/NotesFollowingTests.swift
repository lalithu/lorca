import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class FollowingMockBackend: MockBackend {
    var followRequests: [[String: Any]] = []
    var cancellations: [String] = []
    var research = false
    var kind = "breakdown"
    var failSource: String?
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
        if failSource == fields["text"] as? String {
            failSource = nil
            throw BackendError(message: "A precise test failure")
        }
        let proposal: [String: Any]
        if research && fields["researchQuery"] == nil {
            proposal = ["kind":"research", "source":fields["text"]!, "title":"Finding places in NoDa…", "text":"restaurants in NoDa Charlotte"]
        } else {
            proposal = ["kind":kind, "source":fields["text"]!, "title":"Next steps", "text":"An outline"]
        }
        return try JSONSerialization.data(withJSONObject: ["noteId":fields["noteId"]!, "revision":fields["revision"]!,
            "requestId":fields["requestId"]!, "model":model,
            "suggestions":[proposal]])
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

    func testLatestRevisionWinsAndAddsOnlyItsResultToTheNote() async throws {
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
        XCTAssertEqual(following.proposals.first?.source, "Latest task needs a study guide")
        XCTAssertTrue(notes.current.text.hasPrefix("Latest task needs a study guide"))
        XCTAssertTrue(notes.current.text.contains("An outline"))
        XCTAssertEqual(notes.current.modelBlocks?.count, 1)
        XCTAssertFalse(backend.cancellations.isEmpty)
        XCTAssertEqual(backend.followRequests.count, 2)
        following.setVisible(false)
        XCTAssertEqual(following.proposals.first?.source, "Latest task needs a study guide")
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
        for i in 0..<5 { let text = "Assignment list revision \(i)"; notes.updateText(text, selection: NSRange(location: (text as NSString).length, length: 0)) }
        try await Task.sleep(for: .milliseconds(180))
        XCTAssertEqual(backend.followRequests.count, 1)
        XCTAssertEqual(following.proposals.first?.source, "Assignment list revision 4")
        following.setVisible(false)
    }

    func testOtherPassageEditsAndLeavingNotesDoNotCancelWorkAndResultsPersist() async throws {
        let backend = FollowingMockBackend()
        let suite = "lorca.notes-passages.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 500_000_000)
        following.setVisible(true)
        let first = "Break down on GNN architecture:"
        notes.updateText(first + "\n\n", selection: NSRange(location: (first as NSString).length + 2, length: 0))
        try await Task.sleep(for: .milliseconds(20))
        for i in 0..<4 {
            notes.updateText(first + "\n\nAnother heading I am writing \(i)", selection: NSRange(location: (first as NSString).length + 8, length: 0))
            try await Task.sleep(for: .milliseconds(8))
        }
        XCTAssertEqual(backend.followRequests.first?["text"] as? String, first)
        following.setVisible(false)
        try await Task.sleep(for: .milliseconds(220))
        XCTAssertTrue(backend.cancellations.isEmpty)
        XCTAssertEqual(backend.followRequests.count, 2)
        XCTAssertEqual(notes.current.followingResults?.count, 2)
        XCTAssertTrue(notes.current.followingResults?.allSatisfy(\.unread) == true)
        let reopened = NotesStore(root: backend.root)
        XCTAssertEqual(reopened.current.followingResults, notes.current.followingResults)
        following.setVisible(true)
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(backend.followRequests.count, 2, "Completed passages are not regenerated on every visit")
        following.markRead(source: first)
        XCTAssertFalse(notes.current.followingResults!.first { $0.source == first }!.unread)
    }

    func testRetryIsLocalAndDoesNotRegenerateCompletedPassages() async throws {
        let backend = FollowingMockBackend()
        let first = "Study guide on GNN architecture"
        backend.failSource = first
        let suite = "lorca.notes-retry.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 5_000_000)
        following.setVisible(true)
        let text = first + "\n\nA second independent passage\n\n"
        notes.updateText(text, selection: NSRange(location: (text as NSString).length, length: 0))
        try await Task.sleep(for: .milliseconds(230))
        XCTAssertEqual(following.inline.first { $0.source == first }?.detail, "A precise test failure")
        XCTAssertEqual(backend.followRequests.count, 2)
        following.retry(source: first)
        try await Task.sleep(for: .milliseconds(130))
        XCTAssertEqual(backend.followRequests.count, 3)
        XCTAssertEqual(backend.followRequests.filter { ($0["text"] as? String) == "A second independent passage" }.count, 1)
        XCTAssertEqual(notes.current.followingResults?.count, 2)
        XCTAssertEqual(notes.current.passages.map(\.source), [first, "A second independent passage"])
    }

    func testUndoingAnAutomaticReplacementDoesNotTriggerItAgain() async throws {
        _ = NSApplication.shared
        let backend = FollowingMockBackend(); backend.kind = "priorities"
        let suite = "lorca.notes-undo.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 5_000_000)
        following.setVisible(true)
        let source = "Assignment A tomorrow\nAssignment B tonight"
        notes.updateText(source, selection: NSRange(location: 0, length: 0))
        try await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(notes.current.text, "An outline")
        notes.document(for: notes.selectedID).editor.noteUndo.undo()
        try await Task.sleep(for: .milliseconds(180))
        XCTAssertEqual(notes.current.text, source)
        XCTAssertEqual(backend.followRequests.count, 1, "Undo is the author's decision, not another prompt")
    }

    func testResearchUsesModelActivityAndOnlyThenStartsSearch() async throws {
        let backend = FollowingMockBackend(); backend.research = true
        let suite = "lorca.notes-research.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite); try? FileManager.default.removeItem(at: backend.root) }
        let chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        let notes = NotesStore(root: backend.root)
        let following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 5_000_000)
        following.setVisible(true)
        notes.updateText("Resturants in Noda:", selection: NSRange(location: 0, length: 0))
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(following.inline.first?.label, "Thinking…")
        try await Task.sleep(for: .milliseconds(85))
        XCTAssertEqual(following.inline.first?.label, "Finding places in NoDa…")
        XCTAssertEqual(backend.followRequests.count, 2)
        XCTAssertEqual(backend.followRequests.last?["researchQuery"] as? String, "restaurants in NoDa Charlotte")
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertFalse(following.working)
        XCTAssertEqual(following.proposals.first?.kind, "breakdown")
        XCTAssertTrue(notes.current.text.hasPrefix("Resturants in Noda:"))
        XCTAssertTrue(notes.current.text.contains("An outline"))
    }

    func testPassageBoundariesDoNotRequireCommandsAndPreserveUnicodeOffsets() {
        let text = "Resturants in Noda:\n\nStudy guide for NLP\n- attention\n- tokenization\n\n🐬 Something else entirely"
        let passages = NotesPassage.passages(in: text)
        XCTAssertEqual(passages.count, 3)
        XCTAssertEqual(NotesPassage.passages(in: "Resturants in Noda:\nStudy guide for NLP:").count, 2)
        XCTAssertEqual(NotesPassage.passages(in: "# First heading\nSome details here\n# Second heading").count, 2)
        for passage in passages { XCTAssertEqual((text as NSString).substring(with: passage.range), passage.source) }
        let withTopics = NotesPassage.passages(in: "Study guide for NLP:\n\n- tokenization\n- attention\n\nResturants in Noda:")
        XCTAssertEqual(withTopics.count, 2)
        XCTAssertTrue(withTopics[0].source.contains("- attention"))
        XCTAssertEqual(passages[1].source, "Study guide for NLP\n- attention\n- tokenization")
        XCTAssertTrue(NotesPassage.passages(in: "An ambiguous passage\n\nAn ambiguous passage").isEmpty)
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
