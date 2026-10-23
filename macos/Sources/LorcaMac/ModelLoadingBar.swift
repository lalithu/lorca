import SwiftUI

struct ModelLoadingBar: View {
    let model: String
    let started: Date
    var compact = false

    var body: some View {
        HStack(spacing: 9) {
            ProgressView().progressViewStyle(.linear).controlSize(.mini)
                .frame(width: compact ? 44 : 90)
                .tint(.lorcaBlue)
            Text("Loading \(model)…").lineLimit(1).truncationMode(.middle)
            Spacer(minLength: 0)
            TimelineView(.periodic(from: started, by: 0.1)) { context in
                Text("\(max(0, context.date.timeIntervalSince(started)), specifier: "%.1f")s")
                    .monospacedDigit()
            }
        }
        .font(.system(size: compact ? 9 : 11, design: .monospaced))
        .foregroundStyle(.secondary)
        .accessibilityLabel("Loading \(model)")
    }
}
