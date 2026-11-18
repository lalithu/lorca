import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesWorkingCueTests: XCTestCase {
    func testWorkingCueLeavesWritingAloneAndCannotReturnAfterCancellation() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let editor = NotesNativeTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 400))
        editor.textContainerInset = NSSize(width: 22, height: 0)
        editor.string = "Study guide for NLP\nTransformers and attention"
        editor.setSelectedRange(NSRange(location: 4, length: 0))
        window.contentView = editor
        editor.format()
        let before = editor.string, selection = editor.selectedRange()
        editor.showWorking(true, document: before, at: 4)
        XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
        let shown = expectation(description: "Quiet delayed indicator")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            XCTAssertEqual(editor.subviews.compactMap { $0 as? NotesWorkingCue }.count, 1)
            XCTAssertEqual(editor.string, before)
            XCTAssertEqual(editor.selectedRange(), selection)
            editor.showWorking(false, document: before, at: 4)
            XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
            editor.showWorking(true, document: before, at: 4)
            editor.showWorking(false, document: before, at: 4)
            shown.fulfill()
        }
        wait(for: [shown], timeout: 2)
        let cancelled = expectation(description: "Old timer stays cancelled")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            XCTAssertTrue(editor.subviews.compactMap { $0 as? NotesWorkingCue }.isEmpty)
            cancelled.fulfill()
        }
        wait(for: [cancelled], timeout: 2)
    }

    func testReducedMotionCueIsStaticAndNeverTakesInput() {
        let cue = NotesWorkingCue(reduceMotion: true)
        XCTAssertNil(cue.hitTest(NSPoint(x: 5, y: 5)))
        XCTAssertFalse(cue.acceptsFirstResponder)
        XCTAssertEqual(cue.layer?.sublayers?.count, 3)
        XCTAssertTrue(cue.layer?.sublayers?.allSatisfy { $0.animationKeys()?.isEmpty ?? true } == true)
    }
}
