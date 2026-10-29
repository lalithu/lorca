import Foundation
import AppKit
import Darwin

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

// Keep AppKit's ordinary event loop running while asynchronous cleanup drains.
// Calling terminateLater before this work completes can strand MainActor tasks.
@MainActor
final class AppShutdown {
    enum Phase { case running, closing, ready }
    private(set) var phase: Phase = .running

    func begin(unload: Bool, cleanup: @escaping @MainActor (Bool) async -> [String],
               completed: @escaping @MainActor ([String]) -> Void) {
        guard phase == .running else { return }
        phase = .closing
        Task {
            let failures = await cleanup(unload)
            phase = .ready
            completed(failures)
        }
    }
}

// A per-process marker lets the rebuild helper distinguish cooperative clients
// from older apps, which must be quit normally rather than killed with SIGTERM.
@MainActor
final class RebuildSignal {
    private let source: DispatchSourceSignal
    private let marker: URL

    init(root: URL, request: @escaping @MainActor () -> Void) throws {
        let runtime = root.appendingPathComponent("runtime", isDirectory: true)
        try FileManager.default.createDirectory(at: runtime, withIntermediateDirectories: true)
        marker = runtime.appendingPathComponent("ui-\(ProcessInfo.processInfo.processIdentifier).json")
        let executable = Bundle.main.executableURL!
        let attributes = try FileManager.default.attributesOfItem(atPath: executable.path)
        let data = try JSONSerialization.data(withJSONObject: [
            "pid": ProcessInfo.processInfo.processIdentifier, "protocol": 1,
            "executable": executable.path, "inode": attributes[.systemFileNumber]!
        ])
        source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        signal(SIGTERM, SIG_IGN)
        source.setEventHandler { MainActor.assumeIsolated { request() } }
        source.resume()
        do {
            try data.write(to: marker, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: marker.path)
        } catch {
            source.cancel()
            signal(SIGTERM, SIG_DFL)
            throw error
        }
    }

    func removeMarker() { try? FileManager.default.removeItem(at: marker) }
}
