import AppKit
import SwiftUI
import XCTest
@testable import LorcaMac


@MainActor
final class TextSelectionTests: XCTestCase {
    func testMarkdownSharesOneContinuousSelectableString() {
        let text = MarkdownText.rendered("First **paragraph**.\n\n- List item\n\n```swift\nlet whale = \"🫍\"\n```\n\nLast paragraph.", size: 14)
        XCTAssertEqual(String(text.characters), "First paragraph.\n\n•  List item\n\nlet whale = \"🫍\"\n\nLast paragraph.")
        XCTAssertTrue(text.runs.contains { $0.inlinePresentationIntent?.contains(.stronglyEmphasized) == true })
        XCTAssertTrue(text.runs.contains { $0.font == .system(size: 13, design: .monospaced) })
    }

    func testListsAreCompactAndParagraphsStaySeparate() {
        let source = "A food chain shows how energy passes through eating.\n\n- **Producers:** Plants make food.\n- **Consumers:** Animals eat plants or animals.\n- **Decomposers:** Fungi recycle nutrients.\n\nGrass → rabbit → fox."
        let text = String(MarkdownText.rendered(source, size: 14).characters)
        XCTAssertEqual(text, "A food chain shows how energy passes through eating.\n\n•  Producers: Plants make food.\n•  Consumers: Animals eat plants or animals.\n•  Decomposers: Fungi recycle nutrients.\n\nGrass → rabbit → fox.")
        XCTAssertEqual(String(MarkdownText.rendered("1. First\n2. Second", size: 14).characters), "1.  First\n2.  Second")
    }

    func testNestedListsSeparateSectionsWithoutSpacingEveryBullet() {
        let source = "1. **Producers**\n   - Plants make food.\n   - Example: plankton.\n2. **Consumers**\n   - Animals eat other organisms."
        let text = String(MarkdownText.rendered(source, size: 14).characters)
        XCTAssertTrue(text.contains("•  Plants make food.\n  •  Example: plankton.\n\n2.  Consumers"))
    }

    func testNativeSwiftUIWrappingRecalculatesWhenNarrowedAndWidened() {
        _ = NSApplication.shared
        let text = String(repeating: "A longer paragraph with text that should wrap correctly. ", count: 20)
        let controller = NSHostingController(rootView: MarkdownText(text: text, size: 14))
        let view = controller.view
        func height(_ width: CGFloat) -> CGFloat {
            view.setFrameSize(NSSize(width: width, height: 2000))
            view.layoutSubtreeIfNeeded()
            return controller.sizeThatFits(in: NSSize(width: width, height: 10000)).height
        }
        let wide = height(700), narrow = height(240), wideAgain = height(700)
        XCTAssertGreaterThan(narrow, wide)
        XCTAssertEqual(wideAgain, wide, accuracy: 1)
    }
}
