import XCTest
@testable import LorcaMac

final class BackendProtocol: URLProtocol {
    static var reply: (URLRequest) throws -> (Int, Data) = { _ in (500, Data()) }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        do {
            let (status, data) = try Self.reply(request)
            client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: nil)!, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch { client?.urlProtocol(self, didFailWithError: error) }
    }
    override func stopLoading() {}
}

@MainActor
final class BackendTests: XCTestCase {
    func testAuthenticatedCommandsAndStreamingSnapshotsUseRustEndpoints() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root.appendingPathComponent("runtime"), withIntermediateDirectories: true)
        try Data(#"{"url":"http://127.0.0.1:43210","token":"test-secret","protocol":2}"#.utf8).write(to: root.appendingPathComponent("runtime/endpoint.json"))
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [BackendProtocol.self]
        let network = URLSession(configuration: config)
        defer { network.invalidateAndCancel() }
        let projection = MockBackend()
        let snapshot = try JSONEncoder().encode(projection.snapshot())
        var paths: [String] = []
        BackendProtocol.reply = { request in
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer test-secret")
            let path = request.url!.path
            paths.append(path)
            if path == "/health" { return (200, Data(#"{"protocol":2}"#.utf8)) }
            if path == "/events" {
                XCTAssertEqual(request.value(forHTTPHeaderField: "X-Lorca-Session"), "test-session")
                return (200, snapshot + Data("\n".utf8))
            }
            XCTAssertEqual(path, "/command")
            var object = try JSONSerialization.jsonObject(with: snapshot) as! [String: Any]
            object = ["session": "test-session", "state": object]
            return (200, try JSONSerialization.data(withJSONObject: object))
        }
        let backend = BackendClient(root: root, network: network)
        let state = try await backend.connect(model: "orca")
        XCTAssertEqual(state.conversations.count, 1)
        XCTAssertEqual(backend.sessionID, "test-session")
        var received = 0
        do { try await backend.events { _ in received += 1 } } catch { /* A completed event stream reconnects. */ }
        XCTAssertEqual(received, 1)
        XCTAssertEqual(paths, ["/health", "/command", "/events"])
    }
}

@MainActor
final class AgentBackendTests: XCTestCase {
    func testAgentStreamSharesAuthenticationAndReconnectReusesValidSession() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try FileManager.default.createDirectory(at: root.appendingPathComponent("runtime"), withIntermediateDirectories: true)
        try Data(#"{"url":"http://127.0.0.1:43210","token":"agent-secret","protocol":2}"#.utf8).write(to: root.appendingPathComponent("runtime/endpoint.json"))
        let configuration = URLSessionConfiguration.ephemeral; configuration.protocolClasses = [BackendProtocol.self]
        let network = URLSession(configuration: configuration); defer { network.invalidateAndCancel() }
        let state = try JSONEncoder().encode(MockBackend().snapshot())
        let connected = try JSONSerialization.data(withJSONObject: ["session": "test-session", "state": JSONSerialization.jsonObject(with: state)])
        let agent = try JSONEncoder().encode(agentFixture())
        var commands = 0, streams = 0
        BackendProtocol.reply = { request in
            XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer agent-secret")
            switch request.url!.path {
            case "/health": return (200, Data(#"{"protocol":2}"#.utf8))
            case "/agent/events":
                streams += 1
                XCTAssertEqual(request.value(forHTTPHeaderField: "X-Lorca-Session"), "test-session")
                return (200, agent + Data("\n".utf8))
            default:
                commands += 1
                return (200, commands == 1 ? connected : state)
            }
        }
        let backend = BackendClient(root: root, network: network)
        _ = try await backend.connect(model: "qwen")
        var received: AgentTaskSnapshot?
        do { try await backend.agentEvents { received = $0 } } catch { /* Completed stream reconnects. */ }
        XCTAssertEqual(received?.approval?.id, "approval"); XCTAssertEqual(streams, 1)
        _ = try await backend.connect(model: "qwen")
        XCTAssertEqual(backend.sessionID, "test-session"); XCTAssertEqual(commands, 2)
    }
}
