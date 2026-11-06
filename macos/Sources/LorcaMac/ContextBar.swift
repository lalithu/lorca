import SwiftUI

struct ContextBar: View {
    @ObservedObject var store: ChatStore
    @State private var showingSettings = false
    let usage: ChatUsage
    var compact = false
    var processing = false

    private var used: String { usage.contextUsed.map { $0.formatted() } ?? "—" }
    private var limit: String { usage.contextLimit.map { $0.formatted() } ?? "—" }
    private var percent: String { usage.fraction.map { "\(Int(($0 * 100).rounded()))%" } ?? "—" }

    var body: some View {
        HStack(spacing: compact ? 7 : 9) {
            Button {
                showingSettings.toggle()
            } label: {
                Text(processing ? "ctx · last" : "ctx").fontWeight(.semibold)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Context settings")
            .accessibilityValue("\(percent), \(used) of \(limit) tokens")
            .popover(isPresented: $showingSettings, arrowEdge: .top) {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("Context").font(.headline)
                        Spacer()
                        Text("\(used) / \(limit)").font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                    GeometryReader { geometry in
                        Capsule().fill(.primary.opacity(0.09))
                            .overlay(alignment: .leading) {
                                Capsule().fill(.cyan)
                                    .frame(width: geometry.size.width * (usage.fraction ?? 0))
                            }
                    }.frame(height: 5)
                    HStack {
                        Text("\(percent) last reported use")
                        Spacer()
                        if let n=usage.contextLimit, let u=usage.contextUsed {
                            Text("\(max(0,n-u).formatted()) remaining")
                        }
                    }.font(.caption).foregroundStyle(.secondary)
                    Divider()
                    Text("Working context for next task").fontWeight(.medium)
                    ContextChoices(store: store, allocated: store.allocatedContext, compact: true)
                    Button("Use recommended \(store.recommendedContext.formatted())") {
                        store.contextOverride = nil
                    }.font(.caption).buttonStyle(.link)
                    Divider()
                    Text("Last confirmed inference").font(.caption).foregroundStyle(.secondary)
                    if let count=usage.promptTokens {
                        LabeledContent("Prompt evaluated", value: "\(count.formatted()) tokens")
                    }
                    if let count=usage.generatedTokens {
                        LabeledContent("Tokens generated", value: "\(count.formatted()) tokens")
                    }
                    if let speed=usage.tokensPerSecond {
                        LabeledContent("Generation speed", value: String(format: "%.1f t/s",speed))
                    }
                    Text("These are inference statistics, not live KV-cache measurements. Older turns remain recoverable through Lorca memory.")
                        .font(.caption2).foregroundStyle(.tertiary)
                }
                .padding().frame(width: 360)
            }
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
        .help("Confirmed prompt and reply tokens from the most recent model turn, including tool execution. During generation these are the last confirmed measurements; compaction clears them. — means the local server hasn’t reported a value yet.")
    }
}
