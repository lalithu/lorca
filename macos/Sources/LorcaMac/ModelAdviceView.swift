import SwiftUI

/// Read-only resource and model-fit guidance. Model selection/download remains
/// an explicit user action in the existing model picker or local runtime.
struct ModelAdviceView: View {
    let backend: BackendTransport
    @Environment(\.dismiss) private var dismiss
    @State private var report = "Checking this machine and connected model runtime…"
    @State private var loading = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Label("Local model advisor", systemImage: "cpu")
                    .font(.headline)
                Spacer()
                Button("Close") { dismiss() }
            }
            Text("Hardware-aware estimates and backend measurements, not a benchmark or automatic model change.")
                .font(.caption).foregroundStyle(.secondary)
            ScrollView {
                Text(report)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
                    .padding(12)
            }
            .background(.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
            HStack {
                Button("Refresh installed models") { Task { await update(online: false) } }
                Button("Explore Hugging Face GGUF catalog") { Task { await update(online: true) } }
                Spacer()
                if loading { ProgressView().controlSize(.small) }
            }
            .disabled(loading)
        }
        .padding(18)
        .frame(minWidth: 580, minHeight: 440)
        .task { await update(online: false) }
    }

    @MainActor private func update(online: Bool) async {
        loading = true
        defer { loading = false }
        do {
            let data = try await backend.command(["op": "hardwareAdvice", "online": online])
            guard let result = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let hardware = result["hardware"] as? [String: Any] else {
                report = "Model advisor returned an invalid response."
                return
            }
            func size(_ key: String) -> String {
                guard let value = hardware[key] as? NSNumber else { return "Unknown" }
                return String(format: "%.2f GiB", value.doubleValue / 1_073_741_824)
            }
            var text = """
            PLATFORM: \(hardware["os"] as? String ?? "Unknown")
            CPU THREADS: \(hardware["cpuThreads"] as? Int ?? 0)
            PHYSICAL MEMORY: \(size("physicalMemoryBytes"))
            RECLAIMABLE MEMORY ESTIMATE: \(size("availableMemoryBytes"))
            AVAILABLE STORAGE: \(size("availableStorageBytes"))
            RUNTIME: \(result["runtime"] as? String ?? "Unknown")

            INSTALLED MODELS

            """
            if let context = result["allocatedContext"] as? NSNumber {
                text += "CURRENT ALLOCATED CONTEXT: \(context.intValue) tokens (reported by backend)\n\n"
            } else {
                text += "CURRENT ALLOCATED CONTEXT: unknown; no verified context-fit recommendation\n\n"
            }
            for entry in result["installedModels"] as? [[String: Any]] ?? [] {
                let model = entry["model"] as? String ?? "Unnamed"
                let parameters = entry["parameterSize"] as? String ?? "unknown parameters"
                let quant = entry["quantization"] as? String ?? "unknown quantization"
                let fits = (entry["likelyFitsWeights"] as? Bool).map { $0 ? "likely" : "unlikely" } ?? "unverified"
                text += "\(model) · \(parameters) · \(quant) · new-load weight headroom: \(fits)\n"
                if let confidence = entry["confidence"] as? String { text += "  \(confidence)\n" }
                if let context = entry["context"] as? NSNumber {
                    text += "  Allocated context: \(context.intValue) tokens\n"
                }
                for (key, label) in [("minMemory", "Minimum"), ("maxMemory", "Maximum snapshot"), ("effectiveMemory", "Actual allocation")] {
                    if let measurement = entry[key] as? [String: Any] {
                        let context = (measurement["n_ctx"] as? NSNumber)?.intValue ?? 0
                        text += "  \(label): \(context) context tokens\n"
                        for device in measurement["devices"] as? [[String: Any]] ?? [] {
                            let bytes = ["model", "context", "compute"].reduce(0.0) { $0 + ((device[$1] as? NSNumber)?.doubleValue ?? 0) }
                            text += String(format: "    %@: %.2f GiB (weights + KV + compute)\n", device["name"] as? String ?? "device", bytes / 1_073_741_824)
                        }
                    }
                }
            }
            if online {
                text += "\nHUGGING FACE GGUF DISCOVERY (fit not yet verified)\n\n"
                for item in result["hubCandidates"] as? [[String: Any]] ?? [] {
                    text += "\(item["id"] as? String ?? "Unknown")\n"
                    text += "\(item["url"] as? String ?? "")\n"
                }
                if let warning = result["hubWarning"] as? String {
                    text += "\nCatalog unavailable: \(warning)\n"
                }
            }
            text += "\nAlready-loaded models occupy RAM; headroom estimates describe an additional allocation.\n\nNo models were downloaded or reconfigured. Check model licenses, tool-call support and GGUF memory/context measurements before installing."
            report = text
        } catch {
            report = "Could not retrieve model advice: \(friendlyError(error))"
        }
    }
}
