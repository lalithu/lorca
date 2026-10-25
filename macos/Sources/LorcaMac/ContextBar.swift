import SwiftUI

struct ContextBar: View {
    let usage: ChatUsage
    var compact = false
    var processing = false

    private var used: String { usage.contextUsed.map { $0.formatted() } ?? "—" }
    private var limit: String { usage.contextLimit.map { $0.formatted() } ?? "—" }
    private var percent: String { usage.fraction.map { "\(Int(($0 * 100).rounded()))%" } ?? "—" }

    var body: some View {
        HStack(spacing: compact ? 7 : 9) {
            Text(processing ? "ctx · last" : "ctx").fontWeight(.semibold)
            GeometryReader { geometry in
                Capsule().fill(Color.cyan.opacity(0.15))
                    .overlay(alignment: .leading) {
                        Capsule().fill(Color.cyan)
                            .frame(width: geometry.size.width * (usage.fraction ?? 0))
                    }
            }
            .frame(width: compact ? 44 : 90, height: 4)
            Text("\(percent) · \(used)/\(limit)")
            Spacer(minLength: 0)
            if let speed = usage.tokensPerSecond {
                Text("\(speed, specifier: "%.1f") t/s").foregroundStyle(.secondary)
            }
        }
        .font(.system(size: compact ? 9 : 11, design: .monospaced))
        .monospacedDigit()
        .foregroundStyle(.cyan)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Context: \(percent), \(used) of \(limit) tokens")
        .accessibilityValue(usage.tokensPerSecond.map { String(format: "%.1f tokens per second", $0) } ?? "")
        .help("Confirmed prompt and reply tokens from the most recent model turn, including tool execution. During generation these are the last confirmed measurements; compaction clears them. — means Ollama hasn’t reported a value yet.")
    }
}
