import Foundation

struct BackendSnapshot: Codable {
    let revision: UInt64
    let conversations: [Conversation]
    let sessions: [BackendSession]
    let turns: [BackendTurn]
    let warning: String?
}
struct BackendSession: Codable {
    let id: String
    let selectedId: UUID
    let model: String
    let loadingModel: String?
    let activeModel: String?
    let modelsToRelease: [String]
    let models: [String]
    let isConnected: Bool
    let error: String?
    let usage: ChatUsage
}
struct BackendTurn: Codable {
    let id: UUID
    let conversationId: UUID
    let sessionId: String
    let model: String
    let running: Bool
    let error: String?
    let usage: ChatUsage
}
struct BackendError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

@MainActor
protocol BackendTransport: AnyObject {
    var sessionID: String? { get }
    var root: URL { get }
    func connect(model: String) async throws -> BackendSnapshot
    func command(_ fields: [String: Any]) async throws -> Data
    func events(_ receive: @escaping @MainActor (BackendSnapshot) -> Void) async throws
    func agentEvents(_ receive: @escaping @MainActor (AgentTaskSnapshot?) -> Void) async throws
}

extension BackendTransport {
    func agentEvents(_ receive: @escaping @MainActor (AgentTaskSnapshot?) -> Void) async throws {
        throw BackendError(message: "Agent events are unavailable. Rebuild and relaunch lorca.")
    }
}

@MainActor
final class BackendClient: BackendTransport {
    struct Endpoint: Decodable { let url: URL; let token: String; let `protocol`: Int }
    private(set) var sessionID: String?
    let root: URL
    private let network: URLSession
    private var endpoint: Endpoint?
    private var child: Process?
    init(root: URL? = nil, network: URLSession = .shared) {
        self.root = root ?? Self.defaultRoot
        self.network = network
    }
    static var defaultRoot: URL {
        if let path = ProcessInfo.processInfo.environment["LORCA_DATA_DIR"] { return URL(fileURLWithPath: path) }
        return FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("lorca", isDirectory: true)
    }
    private func discover() async throws {
        let path = root.appendingPathComponent("runtime/endpoint.json")
        var launched = false
        let deadline = Date().addingTimeInterval(5)
        for _ in 0..<100 {
            if Date() > deadline { break }
            if let data = try? Data(contentsOf: path), let candidate = try? JSONDecoder().decode(Endpoint.self, from: data) {
                guard candidate.protocol == 1, candidate.url.scheme == "http", candidate.url.host == "127.0.0.1" else {
                    throw BackendError(message: "Incompatible lorca backend. Quit lorca clients and relaunch.")
                }
                var request = URLRequest(url: candidate.url.appendingPathComponent("health"))
                request.timeoutInterval = 0.3
                request.setValue("Bearer \(candidate.token)", forHTTPHeaderField: "Authorization")
                if let (_, response) = try? await network.data(for: request), (response as? HTTPURLResponse)?.statusCode == 200 {
                    endpoint = candidate
                    return
                }
            }
            if !launched {
                let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/lorca-core")
                guard FileManager.default.isExecutableFile(atPath: executable.path) else {
                    throw BackendError(message: "Rust backend is missing. Rebuild lorca with macos/build.sh.")
                }
                let process = Process()
                process.executableURL = executable
                process.arguments = ["serve"]
                var environment = ProcessInfo.processInfo.environment
                environment["LORCA_DATA_DIR"] = root.path
                process.environment = environment
                process.standardInput = FileHandle.nullDevice
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.nullDevice
                try process.run()
                child = process
                launched = true
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw BackendError(message: "Couldn’t start lorca’s Rust backend. Check Application Support permissions and reconnect.")
    }
    func connect(model: String) async throws -> BackendSnapshot {
        let previous = endpoint
        try await discover()
        if previous?.token == endpoint?.token, sessionID != nil,
           let data = try? await command(["op": "snapshot"]),
           let snapshot = try? JSONDecoder().decode(BackendSnapshot.self, from: data) { return snapshot }
        let data = try await command(["op": "connect", "model": model, "host": "http://127.0.0.1:11434"])
        struct Connected: Decodable { let session: String; let state: BackendSnapshot }
        let result = try JSONDecoder().decode(Connected.self, from: data)
        sessionID = result.session
        return result.state
    }
    func command(_ fields: [String: Any]) async throws -> Data {
        guard let endpoint else { throw BackendError(message: "lorca’s backend is disconnected. Reconnect to continue.") }
        var fields = fields
        if fields["session"] == nil { fields["session"] = sessionID }
        var request = URLRequest(url: endpoint.url.appendingPathComponent("command"))
        request.httpMethod = "POST"
        request.timeoutInterval = 140
        request.setValue("Bearer \(endpoint.token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: fields)
        let (data, response) = try await network.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            let detail = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
            throw BackendError(message: detail ?? "lorca’s backend session ended. Reconnect to continue.")
        }
        return data
    }
    func events(_ receive: @escaping @MainActor (BackendSnapshot) -> Void) async throws {
        guard let endpoint, let sessionID else { throw BackendError(message: "Backend disconnected") }
        var request = URLRequest(url: endpoint.url.appendingPathComponent("events"))
        request.timeoutInterval = 30
        request.setValue("Bearer \(endpoint.token)", forHTTPHeaderField: "Authorization")
        request.setValue(sessionID, forHTTPHeaderField: "X-Lorca-Session")
        let (bytes, response) = try await network.bytes(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw BackendError(message: "Backend session expired") }
        for try await line in bytes.lines where !line.isEmpty {
            try Task.checkCancellation()
            receive(try JSONDecoder().decode(BackendSnapshot.self, from: Data(line.utf8)))
        }
        throw BackendError(message: "Backend connection ended")
    }
    func agentEvents(_ receive: @escaping @MainActor (AgentTaskSnapshot?) -> Void) async throws {
        guard let endpoint, let sessionID else { throw BackendError(message: "Backend disconnected") }
        var request = URLRequest(url: endpoint.url.appendingPathComponent("agent/events"))
        request.timeoutInterval = 30
        request.setValue("Bearer \(endpoint.token)", forHTTPHeaderField: "Authorization")
        request.setValue(sessionID, forHTTPHeaderField: "X-Lorca-Session")
        let (bytes, response) = try await network.bytes(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw BackendError(message: "Agent event connection ended") }
        for try await line in bytes.lines where !line.isEmpty {
            try Task.checkCancellation()
            let data = Data(line.utf8)
            let state = try await Task.detached(priority: .utility) {
                try JSONDecoder().decode(AgentTaskSnapshot?.self, from: data)
            }.value
            receive(state)
        }
        throw BackendError(message: "Agent event connection ended")
    }

}

struct ChatUsage: Codable {
    var contextUsed: Int? = 0
    var contextLimit: Int?
    var tokensPerSecond: Double?
    var fraction: Double? {
        guard let used = contextUsed, let limit = contextLimit, limit > 0 else { return nil }
        return min(1, max(0, Double(used) / Double(limit)))
    }
}
func friendlyError(_ error: Error) -> String {
    if error is URLError { return "Lost connection to lorca’s Rust backend. Reconnect to restore your saved conversation." }
    if error is DecodingError { return "lorca’s backend sent an unreadable response. Rebuild and relaunch lorca." }
    return error.localizedDescription
}
