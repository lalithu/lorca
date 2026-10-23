import Foundation

struct ChatMessage: Identifiable, Equatable, Codable {
    enum Role: String, Codable { case user, assistant }
    let id: UUID
    let role: Role
    var content: String

    init(role: Role, content: String) {
        self.id = UUID()
        self.role = role
        self.content = content
    }
}

struct ChatUsage {
    var contextUsed: Int? = 0
    var contextLimit: Int?
    var tokensPerSecond: Double?

    var fraction: Double? {
        guard let used = contextUsed, let limit = contextLimit, limit > 0 else { return nil }
        return min(1, max(0, Double(used) / Double(limit)))
    }
}

struct OllamaClient {
    let baseURL: URL
    let session: URLSession

    init(baseURL: URL = URL(string: "http://127.0.0.1:11434")!, session: URLSession = .shared) {
        self.baseURL = baseURL
        self.session = session
    }

    func models() async throws -> [String] {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/tags"))
        request.timeoutInterval = 5
        let (data, response) = try await session.data(for: request)
        try validate(response)
        return try JSONDecoder().decode(ModelList.self, from: data).models.map(\.name).sorted()
    }

    func contextLimit(model: String) async throws -> Int? {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/ps"))
        request.timeoutInterval = 1
        let (data, response) = try await session.data(for: request)
        try validate(response)
        let running = try JSONDecoder().decode(RunningModels.self, from: data)
        let limit = running.models.first { $0.name == model || $0.name == "\(model):latest" }?.context_length
        return limit.flatMap { $0 > 0 ? $0 : nil }
    }

    func setKeepAlive(model: String, seconds: Int) async throws {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/generate"))
        request.httpMethod = "POST"
        request.timeoutInterval = seconds == 0 ? 3 : 120
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "model": model, "stream": false, "keep_alive": seconds
        ])
        let (data, response) = try await session.data(for: request)
        let result = try JSONDecoder().decode(ModelAcknowledgement.self, from: data)
        if let error = result.error { throw OllamaError.server(error) }
        try validate(response)
        guard result.done == true else { throw OllamaError.server("Ollama did not confirm the model request.") }
        if let returned = result.model, returned != model && returned != "\(model):latest" {
            throw OllamaError.server("Ollama acknowledged a different model.")
        }
        if seconds == 0 && result.done_reason != "unload" {
            // Older servers can instead confirm that the model is absent from /api/ps.
            var status = URLRequest(url: baseURL.appendingPathComponent("api/ps"))
            status.timeoutInterval = 1
            let (data, response) = try await session.data(for: status)
            try validate(response)
            let running = try JSONDecoder().decode(RunningModels.self, from: data)
            guard !running.models.contains(where: { $0.name == model || $0.name == "\(model):latest" }) else {
                throw OllamaError.server("Ollama did not confirm unloading \(model).")
            }
        }
    }

    func reply(model: String, messages: [ChatMessage], onText: @MainActor (String) -> Void) async throws -> ChatUsage {
        var request = URLRequest(url: baseURL.appendingPathComponent("api/chat"))
        request.httpMethod = "POST"
        request.timeoutInterval = 120
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(ChatRequest(
            model: model,
            messages: messages.map { WireMessage(role: $0.role.rawValue, content: $0.content) }
        ))
        let (bytes, response) = try await session.bytes(for: request)
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            var body = ""
            for try await line in bytes.lines {
                body += line
                if body.count > 4096 { break }
            }
            let detail = body.data(using: .utf8).flatMap { try? JSONDecoder().decode(StreamFrame.self, from: $0).error }
            throw OllamaError.server(detail ?? "Ollama returned HTTP \(http.statusCode).")
        }
        try validate(response)
        for try await line in bytes.lines where !line.isEmpty {
            try Task.checkCancellation()
            let frame = try JSONDecoder().decode(StreamFrame.self, from: Data(line.utf8))
            if let error = frame.error { throw OllamaError.server(error) }
            if let text = frame.message?.content, !text.isEmpty { await onText(text) }
            if frame.done == true {
                let used = frame.prompt_eval_count.flatMap { prompt in
                    frame.eval_count.map { prompt + $0 }
                }
                let speed = frame.eval_count.flatMap { count in
                    frame.eval_duration.flatMap { $0 > 0 ? Double(count) * 1_000_000_000 / $0 : nil }
                }
                // Match the CLI: use the allocated window, not the model's theoretical maximum.
                let limit = try? await contextLimit(model: model)
                return ChatUsage(contextUsed: used, contextLimit: limit, tokensPerSecond: speed)
            }
        }
        throw OllamaError.server("The reply ended unexpectedly. Please try again.")
    }

    private func validate(_ response: URLResponse) throws {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw OllamaError.server("Ollama couldn’t complete the request. Please try again.")
        }
    }
}

private struct ModelList: Decodable {
    struct Model: Decodable { let name: String }
    let models: [Model]
}

private struct RunningModels: Decodable {
    struct Model: Decodable {
        let name: String
        let context_length: Int?
    }
    let models: [Model]
}

private struct WireMessage: Encodable {
    let role: String
    let content: String
}

private struct ChatRequest: Encodable {
    let model: String
    let messages: [WireMessage]
    let stream = true
    let think = false
    let keep_alive = -1
}

private struct ModelAcknowledgement: Decodable {
    let model: String?
    let done: Bool?
    let done_reason: String?
    let error: String?
}

private struct StreamFrame: Decodable {
    struct Message: Decodable { let content: String? }
    let message: Message?
    let done: Bool?
    let error: String?
    let prompt_eval_count: Int?
    let eval_count: Int?
    let eval_duration: Double?
}

enum OllamaError: LocalizedError {
    case server(String)
    var errorDescription: String? {
        switch self { case .server(let message): return message }
    }
}

func friendlyError(_ error: Error) -> String {
    if let error = error as? URLError {
        switch error.code {
        case .cannotConnectToHost, .cannotFindHost, .notConnectedToInternet:
            return "Ollama isn’t available. Start it with ‘ollama serve’, then reconnect."
        case .timedOut:
            return "Ollama took too long to respond. Check that it’s running and try again."
        default: break
        }
    }
    if error is DecodingError { return "Ollama sent an unreadable response. Please try again." }
    return error.localizedDescription
}
