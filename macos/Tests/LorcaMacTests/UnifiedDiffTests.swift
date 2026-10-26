import XCTest
@testable import LorcaMac

final class UnifiedDiffTests: XCTestCase {
    func replacement(_ old: [String], _ new: [String]) -> String {
        "--- file.swift\n+++ file.swift\n@@ -1,\(old.count) +1,\(new.count) @@\n"
            + old.map { "-" + $0 + "\n" }.joined() + new.map { "+" + $0 + "\n" }.joined()
    }
    func testSmallEditOnlyHighlightsRealChangesAndFoldsWithThreeLinesOfContext() {
        let old = (1...40).map { "line \($0)" }
        var new = old; new[10] = "changed line"; new[30] = "another change"
        let diff = UnifiedDiff(replacement(old, new))
        XCTAssertEqual(diff.lines.filter { $0.kind == .addition }.map(\.newLine), [11, 31])
        XCTAssertEqual(diff.lines.filter { $0.kind == .removal }.map(\.oldLine), [11, 31])
        XCTAssertEqual(diff.lines.filter { $0.kind == .context }.count, 38)
        let rows = diff.rows()
        let gaps = rows.compactMap { row -> [AgentDiffLine]? in if case .gap(let lines) = row.content { return lines }; return nil }
        XCTAssertEqual(gaps.map(\.count), [7, 13, 6])
        let shown = rows.compactMap { row -> AgentDiffLine? in if case .line(let line) = row.content { return line }; return nil }
        XCTAssertEqual(shown.filter { $0.kind == .context }.compactMap(\.oldLine), [8, 9, 10, 12, 13, 14, 28, 29, 30, 32, 33, 34])
        XCTAssertEqual(gaps.flatMap { $0 }.count + shown.count, diff.lines.count, "Expanded gaps retain every original line")
    }
    func testUnifiedHunksKeepOffsetsAndHeaderLikeCodeIsNotAFileHeader() {
        let diff = UnifiedDiff("--- a\n+++ b\n@@ -20,3 +20,4 @@\n unchanged\n-old\n+new\n+++ literal code\n end\n@@ -80 +81 @@\n-last\n+updated\n")
        XCTAssertEqual(diff.lines.filter { $0.kind == .addition }.map(\.newLine), [21, 22, 81])
        XCTAssertEqual(diff.lines.first { $0.text == "+++ literal code" }?.kind, .addition)
        XCTAssertEqual(diff.lines.first { $0.text == " end" }?.oldLine, 22)
        XCTAssertEqual(diff.lines.first { $0.text == " end" }?.newLine, 23)
    }
    func testInsertDeleteAndFullReplacement() {
        let inserted = UnifiedDiff("--- /dev/null\n+++ file\n@@ -0,0 +1,2 @@\n+one\n+two\n")
        XCTAssertEqual(inserted.lines.filter { $0.kind == .addition }.map(\.newLine), [1, 2])
        XCTAssertTrue(inserted.lines.filter { $0.kind == .addition }.allSatisfy { $0.oldLine == nil })
        let removed = UnifiedDiff("--- file\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-one\n-two\n")
        XCTAssertEqual(removed.lines.filter { $0.kind == .removal }.map(\.oldLine), [1, 2])
        let changed = UnifiedDiff(replacement(["one", "two"], ["three", "four"]))
        XCTAssertEqual(changed.lines.filter { $0.kind == .context }.count, 0)
        XCTAssertEqual(changed.lines.filter { $0.kind == .addition }.count, 2)
        XCTAssertEqual(changed.lines.filter { $0.kind == .removal }.count, 2)
    }
    func testDuplicateLinesAndMissingFinalNewlineAreNotHidden() {
        let diff = UnifiedDiff(replacement(["same", "same", "old", "same"], ["same", "new", "same", "same"]))
        XCTAssertEqual(diff.lines.filter { $0.kind == .context }.count, 3)
        let newline = UnifiedDiff("--- a\n+++ b\n@@ -1 +1 @@\n-text\n\\ No newline at end of file\n+text\n")
        XCTAssertEqual(newline.lines.filter { $0.kind == .removal }.count, 1)
        XCTAssertEqual(newline.lines.filter { $0.kind == .addition }.count, 1)
        XCTAssertTrue(newline.lines.contains { $0.text == "\\ No newline at end of file" })
    }
    func testLongLinesRemainCompleteAndLargeSmallEditsStayCompact() {
        let long = String(repeating: "x", count: 5000)
        let lines = (0..<10000).map { "row \($0)" }
        var changed = lines; changed[5000] = long
        let diff = UnifiedDiff(replacement(lines, changed))
        XCTAssertEqual(diff.lines.first { $0.kind == .addition }?.text, "+" + long)
        XCTAssertLessThan(diff.rows().count, 20)
    }
}
