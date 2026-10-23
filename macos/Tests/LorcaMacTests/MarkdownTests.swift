import XCTest
@testable import LorcaMac

final class MarkdownTests: XCTestCase {
    func testBlockAndInlineMarkdown() {
        let blocks = MarkdownBlock.parse("### Chapter\n\nA **bold** idea and a [link](https://example.com).\n\n---\n\n> A quote")
        XCTAssertEqual(blocks[0].heading, 3)
        XCTAssertEqual(String(blocks[0].text.characters), "Chapter")
        XCTAssertTrue(blocks[1].text.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
        XCTAssertTrue(blocks[1].text.runs.contains { $0.link != nil })
        XCTAssertTrue(blocks[2].isRule)
        XCTAssertTrue(blocks[3].isQuote)
    }

    func testListsKeepNumberingNestingAndContinuationParagraphs() {
        let blocks = MarkdownBlock.parse("3. Three\n4. Four\n\n- Parent\n\n  Continuation\n  - Child")
        XCTAssertEqual(blocks.compactMap(\.marker), ["3.", "4.", "•", "•"])
        XCTAssertEqual(blocks.last?.listDepth, 2)
        XCTAssertNil(blocks.first(where: { String($0.text.characters) == "Continuation" })?.marker)
    }

    func testFencedCodePreservesMarkdownCharactersWhileStreaming() {
        let source = "```rust\n### literal\nlet x = 1;"
        let blocks = MarkdownBlock.parse(source)
        XCTAssertTrue(blocks[0].isCode)
        XCTAssertTrue(String(blocks[0].text.characters).contains("### literal"))
        XCTAssertNil(blocks[0].heading)
        XCTAssertEqual(MarkdownBlock.parse(source + "\n```\n\n## Next").last?.heading, 2)
    }

    func testTableCellsKeepRowsAndHeader() {
        let cells = MarkdownBlock.parse("| Name | Value |\n|---|---|\n| Orca | 42 |")
        XCTAssertEqual(cells.count, 4)
        XCTAssertEqual(Set(cells.compactMap(\.table)).count, 1)
        XCTAssertEqual(Set(cells.compactMap(\.row)).count, 2)
        XCTAssertEqual(cells.filter(\.isTableHeader).count, 2)
    }
}
