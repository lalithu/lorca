import SwiftUI

/// Confirmed inference measurements. Missing historical data stays hidden.
struct InferenceMetrics: View {
    let usage: ChatUsage?
    var input = false
    var body: some View {
        if let usage {
            let tokens = input ? usage.promptTokens : usage.generatedTokens
            let duration = input ? usage.promptDurationNs : usage.generationDurationNs
            let speed = input ? usage.promptTokensPerSecond : usage.tokensPerSecond
            HStack(spacing: 12) {
                if let tokens { Label("\(tokens) \(input ? "input tokens" : "tokens")", systemImage: "textformat.abc") }
                if let duration { Label(String(format: "%.1fs", Double(duration) / 1_000_000_000), systemImage: "clock") }
                if let speed, speed.isFinite, speed > 0 { Label(String(format: "%.1f t/s", speed), systemImage: "speedometer") }
            }
            .font(.system(size: 10)).foregroundStyle(.secondary).monospacedDigit()
            .help(input ? "Full model input includes instructions, tools and conversation history. Processing time and speed cover the server's uncached input." : "Model generation time and speed reported by the local server.")
        }
    }
}

struct ResponseDetails: View {
    let usage: ChatUsage?
    let model: String?
    var elapsed: Double? = nil
    let text: String
    static func modelName(_ name: String) -> String {
        name.split(separator: "/").last.map(String.init)?.replacingOccurrences(of: "-GGUF", with: "") ?? name
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) { modelTag; InferenceMetrics(usage: usage); total }
                VStack(alignment: .leading, spacing: 7) { modelTag; InferenceMetrics(usage: usage); total }
            }
            if !text.isEmpty {
                Button { NativeClipboard.copy(text) } label: { Image(systemName: "doc.on.doc") }
                    .buttonStyle(.plain).font(.system(size: 12)).foregroundStyle(.secondary)
                    .help("Copy response").accessibilityLabel("Copy whole response")
            }
        }.padding(.top, 7)
    }
    @ViewBuilder private var modelTag: some View {
        if let model, !model.isEmpty {
            Label(Self.modelName(model), systemImage: "shippingbox")
                .font(.system(size: 10, weight: .medium)).lineLimit(1).truncationMode(.middle)
                .padding(.horizontal, 8).padding(.vertical, 5)
                .background(.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 6)).help(model)
        }
    }
    @ViewBuilder private var total: some View {
        if let elapsed { Text(String(format: "%.1fs total", max(0, elapsed)))
            .font(.system(size: 10)).foregroundStyle(.tertiary).monospacedDigit()
            .help("Total elapsed time, including loading, processing, tools and generation.") }
    }
}
