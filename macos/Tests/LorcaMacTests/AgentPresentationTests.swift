import XCTest
@testable import LorcaMac

@MainActor
final class AgentPresentationTests: XCTestCase {
    func event(_ sequence: UInt64, _ kind: String, offset: Int, extra: [String: AgentJSON] = [:]) -> AgentEvent {
        var data = extra
        data["presentation"] = .object(["textBytes": .number(Double(offset)), "startedNanos": .number(1_000_000_000),
            "elapsedMillis": .number(355_000), "clock": .string("mac_continuous")])
        return AgentEvent(id: "run:\(sequence)", sequence: sequence, taskId: "run", kind: kind, data: .object(data))
    }
    func testTimelinePreservesUnicodeStreamingAndRealCompactionOrder() {
        let first = "I’ll inspect 🫍.\n\n", last = "Done."
        let events = [
            event(1, "tool_requested", offset: first.utf8.count, extra: ["callId": .string("read"), "tool": .string("read_file")]),
            event(2, "tool_completed", offset: first.utf8.count, extra: ["callId": .string("read"), "result": .object(["content": .string("real")])]),
            event(3, "context_compacted", offset: first.utf8.count),
            event(4, "tool_requested", offset: first.utf8.count, extra: ["callId": .string("edit"), "tool": .string("apply_patch")])
        ]
        let activities = AgentActivity.make(from: events)
        let timeline = AgentTimelineItem.make(text: first + last, events: events, activities: activities)
        XCTAssertEqual(timeline.count, 5)
        XCTAssertEqual(timeline[0].content, .text("I’ll inspect 🫍."))
        XCTAssertEqual(timeline[1].content, .tools([activities[0]]))
        XCTAssertEqual(timeline[2].content, .compaction)
        XCTAssertEqual(timeline[3].content, .tools([activities[1]]))
        XCTAssertEqual(timeline[4].content, .text(last))
        let streaming = AgentTimelineItem.make(text: first + "D", events: events, activities: activities)
        XCTAssertEqual(streaming.map(\.id), timeline.map(\.id), "Token arrival must retain row identity")
        XCTAssertFalse(AgentTimelineItem.make(text: "Answer", events: [], activities: []).contains { $0.content == .compaction })
    }
    func testSummariesDeduplicateAndDoNotClaimPendingOrFailedEdits() {
        var read = AgentActivity(id: "one", tool: "read_file"); read.completed = true
        var edit = AgentActivity(id: "edit", tool: "apply_patch"); edit.arguments = .object(["path": .string("README.md")])
        XCTAssertEqual(AgentActivity.summary([read, read, edit]), "Read files, File edits")
        edit.completed = true
        XCTAssertEqual(AgentActivity.summary([edit, edit]), "Edited 1 file")
        edit.failed = true
        XCTAssertFalse(AgentActivity.summary([edit]).contains("Edited"))
        XCTAssertTrue(AgentActivity.summary([edit]).contains("failed"))
    }
    func testElapsedRestoresWithoutRestartingAndFreezesAtTerminalEvent() throws {
        let events = [event(1, "task_completed", offset: 0)]
        let first = AgentElapsed(events: events)!, restored = AgentElapsed(events: events)!
        XCTAssertEqual(first.seconds(active: false), 355)
        XCTAssertEqual(restored.seconds(active: false), first.seconds(active: false))
        XCTAssertEqual(AgentElapsed.label(first.seconds(active: false)), "5m 55s")
        XCTAssertEqual(AgentElapsed.label(3661), "1h 1m 1s")
        XCTAssertNil(AgentElapsed(events: []), "Historical tasks without timing must not invent durations")
        let encoded = try JSONEncoder().encode(events)
        XCTAssertEqual(try JSONDecoder().decode([AgentEvent].self, from: encoded), events)
    }
    func testArchivedEventsKeepCompactionAndToolEvidence() throws {
        var message = ChatMessage(role: .assistant, content: "Done")
        message.agentTaskId = "run"; message.agentStatus = "completed"
        message.agentEvents = [event(1, "context_compacted", offset: 0)]
        var chat = Conversation(); chat.messages = [message]
        let store = AgentStore(backend: AgentMockBackend())
        store.restoreArchived([chat], selected: chat.id)
        XCTAssertEqual(store.runs["run"]?.events, message.agentEvents)
        store.restoreArchived([chat], selected: chat.id)
        XCTAssertEqual(store.runs.count, 1)
    }
}
