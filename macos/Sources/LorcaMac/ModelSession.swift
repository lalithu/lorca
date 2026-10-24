import Foundation
import Combine

// Model coordination lives in Rust; this holds only UI progress and actions.
@MainActor
final class ModelSession: ObservableObject {
    @Published private(set) var loadingModel: String?
    @Published private(set) var loadingStarted: Date?
    @Published private(set) var activeModel: String?
    @Published private(set) var error: String?
    private(set) var isClosing = false
    private(set) var modelsToRelease: Set<String> = []
    var approvePreviousUnload: ((String) -> Bool)?
    var prepareAction: ((String, Bool) async -> Bool)?
    var closeAction: ((Bool) async -> [String])?
    var clearAction: (() -> Void)?
    var isLoading: Bool { loadingModel != nil }
    func apply(_ state: BackendSession) {
        if loadingModel != state.loadingModel { loadingStarted = state.loadingModel == nil ? nil : Date() }
        loadingModel = state.loadingModel
        activeModel = state.activeModel
        modelsToRelease = Set(state.modelsToRelease)
        error = state.error
    }
    func clearError() { error = nil; clearAction?() }
    func prepare(_ model: String) -> Task<Bool, Never> {
        let approve = activeModel.flatMap { $0 != model ? approvePreviousUnload?($0) : nil } ?? false
        return Task { await prepareAction?(model, approve) ?? false }
    }
    func close(unload: Bool) async -> [String] {
        isClosing = true
        return await closeAction?(unload) ?? []
    }
}
