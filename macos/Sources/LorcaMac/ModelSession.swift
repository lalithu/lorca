import Foundation
import Combine

@MainActor
final class ModelSession: ObservableObject {
    @Published private(set) var loadingModel: String?
    @Published private(set) var loadingStarted: Date?
    @Published private(set) var activeModel: String?
    @Published private(set) var error: String?
    private(set) var isClosing = false
    private(set) var modelsToRelease: Set<String> = []
    var approvePreviousUnload: ((String) -> Bool)?

    private let client: OllamaClient
    private var loading: Task<Bool, Never>?
    private var operationID: UUID?

    init(client: OllamaClient) { self.client = client }
    var isLoading: Bool { loadingModel != nil }

    func prepare(_ model: String) -> Task<Bool, Never> {
        guard !isClosing else { return Task { false } }
        if loadingModel == model, let loading { return loading }
        if activeModel == model { return Task { true } }
        loading?.cancel()
        let id = UUID()
        operationID = id
        let previous = activeModel
        loadingModel = model
        loadingStarted = Date()
        error = nil
        // Include uncertain/cancelled requests so quit can attempt cleanup too.
        modelsToRelease.insert(model)
        let task = Task { [weak self] in
            guard let self else { return false }
            defer {
                if self.operationID == id {
                    self.loadingModel = nil
                    self.loadingStarted = nil
                    self.loading = nil
                }
            }
            do {
                try await self.client.setKeepAlive(model: model, seconds: -1)
                guard !Task.isCancelled, self.operationID == id else { return false }
                self.activeModel = model
                if let previous, previous != model, self.approvePreviousUnload?(previous) == true {
                    do {
                        try await self.client.setKeepAlive(model: previous, seconds: 0)
                        self.modelsToRelease.remove(previous)
                    } catch {
                        self.error = "The new model is ready, but unloading \(previous) wasn’t confirmed."
                    }
                }
                return true
            } catch {
                if self.operationID == id, !Task.isCancelled { self.error = friendlyError(error) }
                return false
            }
        }
        loading = task
        return task
    }

    func close(unload: Bool) async -> [String] {
        isClosing = true
        operationID = nil
        loading?.cancel()
        loading = nil
        loadingModel = nil
        loadingStarted = nil
        guard unload else { return [] }
        let client = client
        let failed = await withTaskGroup(of: String?.self, returning: [String].self) { group in
            for model in modelsToRelease {
                group.addTask {
                    do {
                        try await client.setKeepAlive(model: model, seconds: 0)
                        return nil
                    } catch { return model }
                }
            }
            var failures: [String] = []
            for await failure in group { if let failure { failures.append(failure) } }
            return failures.sorted()
        }
        modelsToRelease = Set(failed)
        activeModel = nil
        return failed
    }
}
