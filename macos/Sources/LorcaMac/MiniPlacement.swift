import Foundation

struct MiniPlacement {
    let origin: NSPoint
    let belowPet: Bool

    init(pet: NSRect, chatSize: NSSize, screen: NSRect, preferredBelow: Bool) {
        let below = pet.minY - chatSize.height - 6
        let above = pet.maxY + 6
        let fitsBelow = below >= screen.minY
        let fitsAbove = above + chatSize.height <= screen.maxY
        // Keep the current side whenever it fits, avoiding flips after ordinary small drags.
        belowPet = preferredBelow ? fitsBelow : (!fitsAbove && fitsBelow)
        origin = NSPoint(
            x: max(screen.minX, min(pet.midX - chatSize.width / 2, screen.maxX - chatSize.width)),
            y: max(screen.minY, min(belowPet ? below : above, screen.maxY - chatSize.height))
        )
    }
}
