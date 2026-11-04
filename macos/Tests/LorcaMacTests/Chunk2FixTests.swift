import AppKit
import SwiftUI
import XCTest
@testable import LorcaMac

@MainActor final class Chunk2FixTests: XCTestCase {
    func task(_ text: String, events: [AgentEvent] = []) -> AgentTaskSnapshot {
        AgentTaskSnapshot(id: "task", owner: "fixture", conversationId: nil, requestId: nil,
            task: "fixture", workspace: "", status: .completed, events: events, approval: nil,
            result: text, assistantText: text)
    }
    func event(_ sequence: UInt64, offset: Int) -> AgentEvent {
        AgentEvent(id: "event-\(sequence)", sequence: sequence, taskId: "task", kind: "tool_requested",
            data: .object(["callId": .string("call-\(sequence)"), "tool": .string("read_file"),
                "presentation": .object(["textBytes": .number(Double(offset))])]))
    }
    func testLeadingWhitespaceUnicodeAndMultipleToolOffsetsRemainInOriginalCoordinates() {
        let first = " \nBefore 🫍.\n\n", second = "After tool.\n", last = "Done.";
        let raw = first + second + last
        let events = [event(1, offset: first.utf8.count), event(2, offset: (first + second).utf8.count)]
        XCTAssertEqual(AgentPresentation.responseText(task(raw)), raw)
        let timeline = AgentTimelineItem.make(text: AgentPresentation.responseText(task(raw)), events: events, activities: AgentActivity.make(from: events))
        XCTAssertEqual(timeline.map(\.content), [.text("Before 🫍."), .tools([AgentActivity.make(from: events)[0]]), .text("After tool."), .tools([AgentActivity.make(from: events)[1]]), .text("Done.")])
        let streaming = AgentTimelineItem.make(text: first + second + "D", events: events, activities: AgentActivity.make(from: events))
        XCTAssertEqual(streaming.map(\.id), timeline.map(\.id))
        let insideUnicode = event(1, offset: " \nBefore ".utf8.count + 1)
        let boundary = AgentTimelineItem.make(text: raw, events: [insideUnicode], activities: AgentActivity.make(from: [insideUnicode]))
        XCTAssertFalse(boundary.contains { if case .text(let t) = $0.content { return t.contains("�") }; return false })
    }
    func testAuthoritativeClarificationDisplayedOnceWithoutRemovingOtherResponseText() {
        let question = "Which file should I inspect?"
        XCTAssertEqual(AgentPresentation.visibleText(question, question: question), "")
        XCTAssertEqual(AgentPresentation.visibleText("I need a filename.\n\n" + question, question: question), "I need a filename.")
        XCTAssertEqual(AgentPresentation.visibleText("An actual failure remains.", question: question), "An actual failure remains.")
        XCTAssertEqual(AgentPresentation.responseText(task("  " + question)), "  " + question)
    }
    func testDisclosureSurvivesReappearanceAndCachedRunUpdates() {
        var disclosure = ThinkingDisclosure(); disclosure.appear(active: true); disclosure.toggle()
        disclosure.appear(active: true); XCTAssertFalse(disclosure.expanded)
        disclosure.toggle(); disclosure.update(active: false); disclosure.appear(active: false); XCTAssertTrue(disclosure.expanded)
        let store = AgentStore(backend: AgentMockBackend())
        var block = AgentThinkingBlock(id: "reason", state: "running", text: "real", afterSequence: 0, textOffset: 0, truncated: false)
        var run = task(""); run.thinking = [block]; store.apply(run)
        XCTAssertTrue(store.thinkingExpanded(taskID: run.id, block: block))
        store.setThinkingExpanded(false, taskID: run.id, blockID: block.id)
        block.state = "complete"; run.thinking = [block]; store.apply(run)
        XCTAssertFalse(store.thinkingExpanded(taskID: run.id, block: block))
        store.setThinkingExpanded(true, taskID: run.id, blockID: block.id)
        XCTAssertTrue(store.thinkingExpanded(taskID: run.id, block: block))
        for i in 0..<8 {
            store.apply(AgentTaskSnapshot(id: "other-\(i)", owner: "fixture", conversationId: nil, requestId: nil, task: "", workspace: "", status: .completed, events: [], approval: nil, result: ""))
        }
        XCTAssertNil(store.runs[run.id])
        XCTAssertFalse(store.thinkingExpanded(taskID: run.id, block: block), "Manual choices expire with their underlying run")
    }
    func testStageLabelsUseMetadataAndOldPayloadStillDecodes() throws {
        var b = AgentThinkingBlock(id: "r", state: "running", text: "irrelevant generated text", afterSequence: 0, textOffset: 0, truncated: false)
        b.stage = .interpretation; XCTAssertEqual(b.label, "Understanding request")
        b.stage = .execution; XCTAssertEqual(b.label, "Thinking")
        let old = Data(#"{"id":"r","state":"complete","text":"","afterSequence":0,"textOffset":0,"truncated":false}"#.utf8)
        XCTAssertNil(try JSONDecoder().decode(AgentThinkingBlock.self, from: old).stage)
    }
    func testThinkingGreenIsStableAndOnlyAffectsLabel() {
        let color = NSColor(ThinkingControlLabel.enabledColor).usingColorSpace(.sRGB)!
        XCTAssertEqual(color.redComponent, 34.0 / 255, accuracy: 0.001)
        XCTAssertEqual(color.greenComponent, 197.0 / 255, accuracy: 0.001)
        XCTAssertEqual(color.blueComponent, 94.0 / 255, accuracy: 0.001)
    }
    func testReasoningWrapsAtNarrowWidthsWithoutChangingTerminalHorizontalLayout() {
        _ = NSApplication.shared
        let prose = String(repeating: "Reasoning prose must wrap inside its panel. ", count: 40)
        let host = NSHostingController(rootView: ReasoningText(text: prose))
        host.view.setFrameSize(NSSize(width: 320, height: 220)); host.view.layoutSubtreeIfNeeded()
        func scrolls(_ v: NSView) -> [NSScrollView] { (v as? NSScrollView).map { [$0] } ?? v.subviews.flatMap { scrolls($0) } }
        for scroll in scrolls(host.view) {
            XCTAssertLessThanOrEqual(scroll.documentView?.bounds.width ?? 0, 321)
        }
        let rendered = MarkdownText(text: "Hi", size: 14, showsCopyButton: false, userMessage: true)
        XCTAssertTrue(rendered.userMessage); XCTAssertFalse(rendered.showsCopyButton)
    }
}
