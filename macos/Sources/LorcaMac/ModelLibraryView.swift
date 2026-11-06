import SwiftUI
import Combine

// Presentation-only state: the Rust core and native runtimes own actual model
// inventory, transfers, allocations and file deletion.
struct LibraryModel: Identifiable {
    let id: String
    let name: String
    let provider: String
    let sizeBytes: UInt64?
    let parameters: String?
    let quant: String?
    let vision: Bool?
    let status: String
    let allocatedContext: Int?
    let loadedMemoryBytes: UInt64?
    let canDelete: Bool

    init(_ value: [String: Any]) {
        id = value["id"] as? String ?? ""
        name = value["name"] as? String ?? id
        provider = value["provider"] as? String ?? ""
        sizeBytes = (value["sizeBytes"] as? NSNumber)?.uint64Value
        parameters = value["parameters"] as? String ?? (value["parameters"] as? NSNumber)?.stringValue
        quant = value["quant"] as? String
        vision = value["vision"] as? Bool
        status = value["status"] as? String ?? "installed"
        allocatedContext = (value["allocatedContext"] as? NSNumber)?.intValue
        loadedMemoryBytes = (value["loadedMemoryBytes"] as? NSNumber)?.uint64Value
        canDelete = value["canDelete"] as? Bool ?? true
    }
}
struct HubModel: Identifiable {
    let id: String
    let name: String
    let parameters: String?
    let recommendedQuant: String?
    let sizeBytes: UInt64?
    let vision: Bool
    let fitStatus: String
    let estimatedFit: Bool
    let builds: [HubBuild]
    let url: URL?

    init(_ value: [String: Any]) {
        id = value["repo"] as? String ?? ""
        name = value["name"] as? String ?? id
        parameters = value["parameters"] as? String
        recommendedQuant = value["recommendedQuant"] as? String
        sizeBytes = (value["sizeBytes"] as? NSNumber)?.uint64Value
        vision = value["vision"] as? Bool ?? false
        fitStatus = value["fitStatus"] as? String ?? "Unknown"
        estimatedFit = value["estimatedFit"] as? Bool ?? false
        builds = (value["builds"] as? [[String: Any]] ?? []).map(HubBuild.init)
        url = (value["url"] as? String).flatMap(URL.init(string:))
    }
}
struct HubBuild: Identifiable {
    let quant: String
    let sizeBytes: UInt64?
    var id: String { quant }
    init(_ value: [String: Any]) {
        quant = value["quant"] as? String ?? "Unknown"
        sizeBytes = (value["sizeBytes"] as? NSNumber)?.uint64Value
    }
}
struct ModelTransfer: Identifiable {
    let id: String
    let status: String
    let completed: UInt64
    let total: UInt64
    let bytesPerSecond: Double
    let error: String?
    init(_ value: [String: Any]) {
        id = value["model"] as? String ?? ""
        status = value["status"] as? String ?? "starting"
        completed = (value["completed"] as? NSNumber)?.uint64Value ?? 0
        total = (value["total"] as? NSNumber)?.uint64Value ?? 0
        bytesPerSecond = (value["bytesPerSecond"] as? NSNumber)?.doubleValue ?? 0
        error = value["error"] as? String
    }
    var fraction: Double { total == 0 ? 0 : min(1, Double(completed) / Double(total)) }
}
@MainActor final class ModelLibraryController: ObservableObject {
    let backend: BackendTransport
    @Published private(set) var installed: [LibraryModel] = []
    @Published private(set) var candidates: [HubModel] = []
    @Published private(set) var transfers: [ModelTransfer] = []
    @Published private(set) var provider = ""
    @Published var searching = false
    @Published var working = false
    @Published var issue: String?
    @Published var search = ""
    @Published var selected: String?
    @Published var selectedQuant: String?
    private var pendingSearch: Task<Void, Never>?

    init(backend: BackendTransport) { self.backend = backend }
    private func response(_ operation: String, extra: [String: Any] = [:]) async throws -> [String: Any] {
        var fields = extra
        fields["op"] = operation
        let data = try await backend.command(fields)
        return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    }
    func refresh() async {
        guard !working else { return }
        do {
            let v = try await response("modelLibrary")
            provider = v["provider"] as? String ?? ""
            installed = (v["installed"] as? [[String: Any]] ?? []).map(LibraryModel.init)
            transfers = (v["downloads"] as? [[String: Any]] ?? []).map(ModelTransfer.init)
            issue = nil
        } catch { issue = "Cannot read models: \(error.localizedDescription)" }
    }
    func searchHub() async {
        searching = true
        defer { searching = false }
        do {
            let v = try await response("modelCatalog", extra: ["query": search])
            candidates = (v["items"] as? [[String: Any]] ?? []).map(HubModel.init)
            issue = nil
        } catch { issue = "Hugging Face search failed: \(error.localizedDescription)" }
    }
    func scheduleSearch() {
        pendingSearch?.cancel()
        pendingSearch = Task {
            try? await Task.sleep(for: .milliseconds(430))
            guard !Task.isCancelled else { return }
            await searchHub()
        }
    }
    func inspect(_ model: HubModel) async {
        selected = model.id
        selectedQuant = model.recommendedQuant
        do {
            let v = try await response("modelDetails", extra: ["repo": model.id])
            let detail = HubModel(v)
            if let index = candidates.firstIndex(where: { $0.id == model.id }) {
                candidates[index] = detail
            }
            if selectedQuant == nil { selectedQuant = detail.recommendedQuant }
        } catch { issue = "Couldn't inspect GGUF files: \(error.localizedDescription)" }
    }
    func transfer(for model: String) -> ModelTransfer? { transfers.first { $0.id == model } }
    func install(_ model: HubModel) async {
        guard let quant = selectedQuant ?? model.recommendedQuant, !working else { return }
        // Runtime-specific Hugging Face IDs; original files belong to each runtime.
        let runtimeID = provider == "ollama"
            ? "hf.co/\(model.id):\(quant)" : "\(model.id):\(quant)"
        working = true
        defer { working = false }
        do {
            _ = try await response("modelInstall", extra: ["model": runtimeID])
            issue = nil
            await refresh()
        } catch { issue = "Installation couldn't start: \(error.localizedDescription)" }
    }
    func cancel(_ model: String) async {
        do {
            _ = try await response("modelCancelDownload", extra: ["model": model])
            await refresh()
        } catch { issue = "Can't cancel download: \(error.localizedDescription)" }
    }
    func delete(_ model: String) async {
        guard !working else { return }
        working = true
        defer { working = false }
        do {
            _ = try await response("modelDelete", extra: ["model": model])
            await refresh()
        } catch { issue = "Can't delete model: \(error.localizedDescription)" }
    }
    func refreshTransfers() async {
        guard let v = try? await response("modelDownloads") else { return }
        transfers = (v["downloads"] as? [[String: Any]] ?? []).map(ModelTransfer.init)
    }
}

private func modelSize(_ bytes: UInt64?) -> String {
    guard let bytes else { return "Size unknown" }
    return ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file)
}
private enum LibraryTab: String, CaseIterable {
    case recommended = "Recommended", installed = "Installed", discover = "Discover"
}
struct ModelLibraryView: View {
    @ObservedObject var store: ChatStore
    @StateObject private var library: ModelLibraryController
    @State private var tab: LibraryTab = .recommended
    @State private var pendingDelete: LibraryModel?
    @State private var selectedInstalled: String?
    @Environment(\.openURL) private var openURL

    init(store: ChatStore) {
        self.store = store
        _library = StateObject(wrappedValue: ModelLibraryController(backend: store.backend))
    }
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                header
                Picker("Library view", selection: $tab) {
                    ForEach(LibraryTab.allCases, id: \.self) { item in
                        Text(item.rawValue).tag(item)
                    }
                }
                .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 450)
                if let message = library.issue {
                    Label(message, systemImage: "exclamationmark.triangle")
                        .foregroundStyle(.orange).font(.caption)
                        .textSelection(.enabled)
                }
                switch tab {
                case .recommended:
                    recommended
                case .installed:
                    installed
                case .discover:
                    discover
                }
            }
            .frame(maxWidth: 970, alignment: .leading)
            .padding(.horizontal, 32).padding(.bottom, 40)
            .frame(maxWidth: .infinity, alignment: .top)
        }
        .task {
            await library.refresh()
            await library.searchHub()
        }
        .task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                if Task.isCancelled { return }
                await library.refreshTransfers()
            }
        }
        .confirmationDialog("Delete \(pendingDelete?.name ?? "model")?", isPresented: Binding(
            get: { pendingDelete != nil },
            set: { if !$0 { pendingDelete = nil } }
        )) {
            if let model = pendingDelete {
                Button("Delete \(modelSize(model.sizeBytes)) of model files", role: .destructive) {
                    Task { await library.delete(model.id); await store.refreshConnection() }
                    pendingDelete = nil
                }
            }
            Button("Cancel", role: .cancel) { pendingDelete = nil }
        } message: {
            Text("This permanently deletes this runtime's local model files. Other apps may also depend on this installation.")
        }
    }
    private var header: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 7) {
                Text("Model Library").font(.system(size: 28, weight: .semibold))
                Text("One catalog. Ollama and llama.cpp. Everything stays on your Mac after download.")
                    .font(.subheadline).foregroundStyle(.secondary)
            }
            Spacer()
            Button { Task { await library.refresh(); await library.searchHub() } } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            }.buttonStyle(.bordered).controlSize(.small)
        }
    }
    private var recommended: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Recommended for your Mac").font(.headline)
            Text("Highest-precision GGUF variants with known file sizes inside a conservative memory budget. Agent performance is not yet benchmarked.")
                .font(.caption).foregroundStyle(.secondary)
            candidateList(library.candidates.filter(\.estimatedFit))
            if library.searching { ProgressView("Checking Hugging Face models…") }
            if !library.searching && library.candidates.filter(\.estimatedFit).isEmpty {
                Text("No verified-size candidates yet. Use Discover to inspect a model's exact GGUF files.")
                    .foregroundStyle(.secondary).font(.subheadline)
            }
        }
    }
    private var installed: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Installed models").font(.headline)
                Spacer()
                Text(library.provider).foregroundStyle(.secondary).font(.caption)
            }
            if library.installed.isEmpty {
                Text("No models reported by the connected runtime. Browse Hugging Face to install one.")
                    .foregroundStyle(.secondary)
            }
            ForEach(library.installed) { model in
                VStack(spacing: 0) {
                    Button {
                        selectedInstalled = selectedInstalled == model.id ? nil : model.id
                    } label: {
                        HStack(spacing: 14) {
                            Image(systemName: model.vision == true ? "eye" : "cube.transparent")
                                .frame(width: 30).font(.title3).foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack(spacing: 8) {
                                    Text(model.name).fontWeight(.semibold).lineLimit(1)
                                    if let parameter = model.parameters { smallChip(parameter) }
                                    if let quant = model.quant { smallChip(quant) }
                                    if model.vision == true { Image(systemName: "eye").accessibilityLabel("Vision") }
                                }
                                Text("\(modelSize(model.sizeBytes)) · \(model.status) · \(model.allocatedContext.map { "\($0.formatted()) ctx" } ?? "Context not reported")")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: selectedInstalled == model.id ? "chevron.up" : "chevron.down")
                                .foregroundStyle(.secondary)
                        }
                        .contentShape(Rectangle())
                        .padding(15)
                    }.buttonStyle(.plain)
                    if selectedInstalled == model.id { installedDetail(model) }
                }
                .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 13))
                .overlay(RoundedRectangle(cornerRadius: 13).stroke(.primary.opacity(0.08)))
            }
        }
    }
    private var discover: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 9) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search Hugging Face GGUF repositories", text: $library.search)
                    .textFieldStyle(.plain).onChange(of: library.search) { _, _ in library.scheduleSearch() }
                if library.searching { ProgressView().controlSize(.small) }
            }
            .padding(12).background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10))
            Text("Installations use your connected runtime's native download manager.")
                .font(.caption).foregroundStyle(.secondary)
            candidateList(library.candidates)
        }
    }
    private func candidateList(_ items: [HubModel]) -> some View {
        VStack(spacing: 9) {
            ForEach(items) { model in
                VStack(alignment: .leading, spacing: 10) {
                    Button { Task { await library.inspect(model) } } label: {
                        HStack(spacing: 12) {
                            Image(systemName: model.vision ? "eye" : "cpu")
                                .foregroundStyle(.secondary).frame(width: 28)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack(spacing: 7) {
                                    Text(model.name).fontWeight(.semibold).lineLimit(1)
                                    if let p = model.parameters { smallChip(p) }
                                    if let q = model.recommendedQuant { smallChip(q) }
                                    if model.vision { Image(systemName: "eye").accessibilityLabel("Vision") }
                                }
                                Text("\(modelSize(model.sizeBytes)) · \(model.fitStatus)")
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            }
                            Spacer()
                            Image(systemName: library.selected == model.id ? "chevron.up" : "chevron.down")
                                .foregroundStyle(.secondary)
                        }.contentShape(Rectangle())
                    }.buttonStyle(.plain)
                    if library.selected == model.id {
                        Divider()
                        Text("Quantization").font(.caption).foregroundStyle(.secondary)
                        if model.builds.isEmpty {
                            Text("GGUF file sizes unavailable. No install variant can be selected safely.")
                                .font(.caption).foregroundStyle(.secondary)
                        } else {
                            Picker("Quantization", selection: Binding(
                                get: { library.selectedQuant ?? model.recommendedQuant ?? model.builds[0].quant },
                                set: { library.selectedQuant = $0 }
                            )) {
                                ForEach(model.builds) { build in
                                    Text("\(build.quant) · \(modelSize(build.sizeBytes))").tag(build.quant)
                                }
                            }.labelsHidden().frame(maxWidth: 360, alignment: .leading)
                        }
                        HStack {
                            if let url = model.url {
                                Button("View on Hugging Face") { openURL(url) }
                                    .buttonStyle(.link)
                            }
                            Spacer()
                            Button {
                                Task { await library.install(model) }
                            } label: {
                                Label("Download to \(library.provider)", systemImage: "arrow.down.circle")
                            }.buttonStyle(.borderedProminent)
                                .disabled(library.working || model.builds.isEmpty)
                        }
                        Text("The quantization is an initial choice. Context capacity, actual KV memory and agent quality need runtime verification.")
                            .font(.caption2).foregroundStyle(.tertiary)
                    }
                }
                .padding(15)
                .background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 13))
                .overlay(RoundedRectangle(cornerRadius: 13).stroke(.primary.opacity(0.08)))
            }
        }
    }
    private func installedDetail(_ model: LibraryModel) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider()
            HStack {
                Text("Context window").fontWeight(.medium)
                Spacer()
                Text(model.allocatedContext.map { "\($0.formatted()) allocated by backend" } ?? "Unverified")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Text("Change your next-task working window in Chat. Ollama may reallocate per request; llama.cpp requires a compatible loaded context. This panel never claims a change succeeded before the runtime reports it.")
                .font(.caption).foregroundStyle(.secondary)
            if model.id == store.selectedModel {
                ContextChoices(store: store, allocated: model.allocatedContext, compact: false)
            }
            HStack(spacing: 12) {
                Button {
                    store.selectedModel = model.id
                    Task { _ = await store.modelSession.prepare(model.id) }
                } label: { Label("Use in chat", systemImage: "bubble.left") }
                    .disabled(!store.ready || store.isManagingModel)
                if model.status != "loaded" {
                    Button("Load") {
                        store.selectedModel = model.id
                        Task { _ = await store.modelSession.prepare(model.id) }
                    }.disabled(!store.canReloadModel)
                } else {
                    Button("Unload") { Task { await store.unloadModel(model: model.id) } }
                        .disabled(!store.canReloadModel)
                }
                Spacer()
                Button(role: .destructive) { pendingDelete = model } label: {
                    Label("Delete", systemImage: "trash")
                }.disabled(!model.canDelete || library.working || store.isManagingModel)
            }.buttonStyle(.bordered)
        }
        .padding(.horizontal, 15).padding(.bottom, 15)
    }
    private func smallChip(_ value: String) -> some View {
        Text(value).font(.system(size: 10, weight: .medium, design: .monospaced))
            .padding(.horizontal, 6).padding(.vertical, 3)
            .background(.primary.opacity(0.08), in: Capsule())
    }
}
