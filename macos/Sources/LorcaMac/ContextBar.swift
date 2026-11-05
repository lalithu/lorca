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
                VStack(alignment: .leading, spacing: 10) {
                    Text("Context window").font(.headline)
                    LabeledContent("Server allocation", value: store.allocatedContext?.formatted() ?? "Not reported")
                    LabeledContent("Next task", value: store.workingContext.formatted())
                    Picker("Working window", selection: $store.contextOverride) {
                        Text("Recommended (\(store.recommendedContext.formatted()))").tag(nil as Int?)
                        ForEach([2048, 4096, 8192, 16384, 32768, 65536, 131072], id: \.self) { size in
                            Text(size.formatted()).tag(Optional(size))
                        }
                    }
                    Text("Recommended starts with a conservative target based on this Mac’s memory. Lorca caps requests at the reported server allocation. Changes apply to the next task.")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding().frame(width: 320)
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
