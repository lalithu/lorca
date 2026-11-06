import SwiftUI

/// Model-facing working context is separate from the backend allocation.
/// The picker never claims a request has changed the running server yet.
struct ContextChoices: View {
    @ObservedObject var store: ChatStore
    let allocated: Int?
    var compact = false
    private let tiers = [4096, 8192, 16384, 32768, 65536, 131072]

    private func short(_ value: Int) -> String {
        value >= 1024 ? "\(value / 1024)K" : "\(value)"
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 4) {
                ForEach(tiers, id: \.self) { value in
                    Button {
                        store.contextOverride = value
                    } label: {
                        VStack(spacing: 3) {
                            Text(short(value)).font(.system(size: compact ? 10 : 12, weight: .semibold))
                            if let allocated, value == allocated {
                                Text("Loaded").font(.system(size: 9)).foregroundStyle(.secondary)
                            }
                        }
                        .frame(maxWidth: .infinity, minHeight: compact ? 25 : 38)
                        .background(store.contextOverride == value ? Color.accentColor.opacity(0.20) : .clear,
                                    in: RoundedRectangle(cornerRadius: 7))
                    }.buttonStyle(.plain)
                }
            }
            .padding(4)
            .background(.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 9))
            HStack {
                Text("Next task: \(short(store.contextOverride ?? store.recommendedContext)) requested")
                Spacer()
                Text("Backend: \(allocated.map(short) ?? "not reported")")
            }
            .font(.caption2).foregroundStyle(.secondary)
            Text(store.providerName == "Ollama"
                 ? "Ollama receives the requested num_ctx on the next inference. Confirm actual allocation afterward."
                 : "llama.cpp uses its loaded context. A larger selection cannot take effect until the runtime is loaded at that size.")
                .font(.caption2).foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
