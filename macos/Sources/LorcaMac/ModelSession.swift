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

    func clearError() { error = nil }

    func prepare(_ model: String) -> Task<Bool, Never> {
        guard !isClosing else { return Task { false } }
        if loadingModel == model, let loading { return loading }
        if activeModel == model && loading == nil { error = nil; return Task { true } }
        let preceding = loading
        preceding?.cancel()
        let id = UUID()
        operationID = id
        loadingModel = model
        loadingStarted = Date()
        error = nil
        // Include uncertain/cancelled requests so quit can attempt cleanup too.
        let task = Task { [weak self] in
            guard let self else { return false }
            defer {
                if self.operationID == id {
                    self.loadingModel = nil
                    self.loadingStarted = nil
                    self.loading = nil
                }
            }
            if let preceding { _ = await preceding.value }
            guard !Task.isCancelled, self.operationID == id else { return false }
            if self.activeModel == model { return true }
            let previous = self.activeModel
            var releasedPrevious = false
            do {
                // With explicit consent, release the old allocation before loading another.
                if let previous, self.approvePreviousUnload?(previous) == true {
                    try await self.client.setKeepAlive(model: previous, seconds: 0)
                    self.modelsToRelease.remove(previous)
                    self.activeModel = nil
                    releasedPrevious = true
                    try Task.checkCancellation()
                }
                self.modelsToRelease.insert(model)
                try await self.client.setKeepAlive(model: model, seconds: -1)
                guard !Task.isCancelled, self.operationID == id else { return false }
                self.activeModel = model
                return true
            } catch {
                if self.operationID == id, !Task.isCancelled {
                    var failure = friendlyError(error)
                    if releasedPrevious, let previous {
                        self.modelsToRelease.insert(previous)
                        do {
                            try await self.client.setKeepAlive(model: previous, seconds: -1)
                            if self.operationID == id, !Task.isCancelled {
                                self.modelsToRelease.insert(previous)
                                self.activeModel = previous
                            }
                        } catch { failure += " The previous model couldn’t be restored: \(friendlyError(error))" }
                    }
                    if self.operationID == id, !Task.isCancelled { self.error = failure }
                }
                return false
            }
        }
        loading = task
        return task
    }

    func close(unload: Bool) async -> [String] {
        isClosing = true
        operationID = nil
        let pending = loading
        pending?.cancel()
        loading = nil
        loadingModel = nil
        loadingStarted = nil
        if let pending { _ = await pending.value }
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
