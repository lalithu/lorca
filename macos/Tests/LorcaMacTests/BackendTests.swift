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
        try Data(#"{"url":"http://127.0.0.1:43210","token":"test-secret","protocol":1}"#.utf8).write(to: root.appendingPathComponent("runtime/endpoint.json"))
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
            if path == "/health" { return (200, Data(#"{"protocol":1}"#.utf8)) }
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
