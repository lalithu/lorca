import Foundation
import Darwin

// The existing Python tracker is a small stdio companion, never a web dashboard.
// Blocking pipe I/O stays off the main actor. All requests are serialized here.
actor JobAppsBridge: JobAppsTransport {
    nonisolated private let lifetime = JobAppsLifetime()
    private let serviceURL: URL?
    init(serviceURL: URL? = nil) { self.serviceURL = serviceURL }
    private var child: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var pending = Data()
    func request(_ payload: Data) throws -> Data {
        guard !lifetime.closed else { throw BackendError(message: "Job Apps is closing") }
        if child?.isRunning != true {
            let resources = Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/jobapps")
            let script = serviceURL ?? ProcessInfo.processInfo.environment["LORCA_JOBS_SERVICE"].map { URL(fileURLWithPath: $0) } ?? resources.appendingPathComponent("service.py")
            guard FileManager.default.fileExists(atPath: script.path) else {
                throw BackendError(message: "Job Apps resources are missing. Rebuild Lorca.")
            }
            let process = Process(), send = Pipe(), receive = Pipe()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
            process.arguments = ["python3", "-B", script.path]
            var environment = ProcessInfo.processInfo.environment
            environment["LORCA_CORE"] = environment["LORCA_CORE"] ?? Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/lorca-core").path
            process.environment = environment
            process.standardInput = send; process.standardOutput = receive; process.standardError = FileHandle.nullDevice
            try lifetime.launch(process); child = process; input = send.fileHandleForWriting; output = receive.fileHandleForReading; pending = Data()
        }
        try input?.write(contentsOf: payload + Data([10]))
        while !pending.contains(10) {
            guard let chunk = output?.availableData, !chunk.isEmpty else { throw BackendError(message: "Job Apps adapter stopped. Reopen the page to reconnect.") }
            pending.append(chunk)
            guard pending.count < 32_000_000 else { throw BackendError(message: "Job Apps response is too large") }
        }
        let end = pending.firstIndex(of: 10)!, data = pending[..<end]
        pending.removeSubrange(...end)
        return Data(data)
    }
    nonisolated func close() async {
        guard let process = lifetime.stop() else { return }
        // A healthy companion has already acknowledged quit and finished its
        // Google cleanup. This path also breaks a stuck pipe without waiting on
        // the actor that is blocked reading it.
        if process.isRunning { process.terminate() }
        for _ in 0..<20 {
            if !process.isRunning { return }
            try? await Task.sleep(for: .milliseconds(100))
        }
        if process.isRunning { kill(process.processIdentifier, SIGKILL) }
    }
}

// Only process lifetime crosses the bridge actor. Pipe protocol state stays
// serialized; closing can still interrupt a non-responsive companion.
private final class JobAppsLifetime: @unchecked Sendable {
    private let lock = NSLock()
    private var process: Process?
    private var stopped = false
    var closed: Bool { lock.lock(); defer { lock.unlock() }; return stopped }
    func launch(_ process: Process) throws {
        lock.lock(); defer { lock.unlock() }
        guard !stopped else { throw BackendError(message: "Job Apps is closing") }
        try process.run()
        self.process = process
    }
    func stop() -> Process? {
        lock.lock(); defer { lock.unlock() }
        stopped = true
        return process
    }
}
