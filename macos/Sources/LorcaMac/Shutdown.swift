import Foundation

// Unlike a task-group race, this deadline does not wait for an unresponsive
// operation to acknowledge cancellation before allowing the app to quit.
@MainActor
func finishWithin<T>(_ seconds: TimeInterval, operation: @escaping @MainActor () async -> T) async -> T? {
    let race = ShutdownRace<T>()
    return await withCheckedContinuation { continuation in
        race.continuation = continuation
        race.work = Task { race.finish(await operation()) }
        race.timer = Task {
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
            race.finish(nil)
        }
    }
}

@MainActor
private final class ShutdownRace<T> {
    var continuation: CheckedContinuation<T?, Never>?
    var work: Task<Void, Never>?
    var timer: Task<Void, Never>?
    func finish(_ value: T?) {
        guard let continuation else { return }
        self.continuation = nil
        work?.cancel(); timer?.cancel()
        work = nil; timer = nil
        continuation.resume(returning: value)
    }
}
