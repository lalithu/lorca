import XCTest
@testable import LorcaMac

final class MiniPlacementTests: XCTestCase {
    private let screen = NSRect(x: 0, y: 0, width: 1440, height: 1000)
    private let chat = NSSize(width: 384, height: 400)

    func testKeepsCurrentSideWhenBothSidesFit() {
        let pet = NSRect(x: 500, y: 410, width: 160, height: 168)
        let above = MiniPlacement(pet: pet, chatSize: chat, screen: screen, preferredBelow: false)
        let below = MiniPlacement(pet: pet, chatSize: chat, screen: screen, preferredBelow: true)
        XCTAssertFalse(above.belowPet)
        XCTAssertTrue(below.belowPet)
    }

    func testOrdinaryMovementPreservesRelativePosition() {
        let pet = NSRect(x: 500, y: 100, width: 160, height: 168)
        let start = MiniPlacement(pet: pet, chatSize: chat, screen: screen, preferredBelow: false)
        let end = MiniPlacement(pet: pet.offsetBy(dx: 90, dy: 60), chatSize: chat,
            screen: screen, preferredBelow: start.belowPet)
        XCTAssertEqual(end.origin.x - start.origin.x, 90)
        XCTAssertEqual(end.origin.y - start.origin.y, 60)
        XCTAssertEqual(end.belowPet, start.belowPet)
    }

    func testSwitchesSideOnlyWhenNecessaryAndStaysOnScreen() {
        for pet in [NSRect(x: -50, y: 20, width: 160, height: 168),
                    NSRect(x: 1380, y: 800, width: 160, height: 168)] {
            let placement = MiniPlacement(pet: pet, chatSize: chat, screen: screen,
                preferredBelow: pet.minY < 100)
            XCTAssertEqual(placement.belowPet, pet.minY > 100)
            XCTAssertTrue(screen.contains(NSRect(origin: placement.origin, size: chat)))
        }
    }
}
