import SwiftUI

enum ChatAccent {
    static let reasoning = Color(nsColor: .systemPurple)
    static let activity = Color.lorcaBlue
    static let approval = Color(nsColor: .systemOrange)
    static let success = Color(nsColor: .systemGreen)
    static let error = Color(nsColor: .systemRed)
}

extension View {
    @ViewBuilder func chatGlass(radius: CGFloat, tint: Color? = nil) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius)
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            glassEffect(.regular.tint(tint), in: shape)
        } else {
            background(.regularMaterial, in: shape)
                .background(tint ?? .clear, in: shape)
        }
        #else
        background(.regularMaterial, in: shape)
            .background(tint ?? .clear, in: shape)
        #endif
    }

    func chatResponseSurface() -> some View {
        padding(.horizontal, 14).padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .chatGlass(radius: 16)
    }

    func chatInlineAccent(_ color: Color) -> some View {
        background(color.opacity(0.045), in: RoundedRectangle(cornerRadius: 8))
            .overlay(alignment: .leading) {
                Capsule().fill(color.opacity(0.42)).frame(width: 2)
                    .padding(.vertical, 5).allowsHitTesting(false)
            }
    }
}
