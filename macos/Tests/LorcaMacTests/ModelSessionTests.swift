import XCTest
@testable import LorcaMac

@MainActor
final class ModelSessionTests: XCTestCase {
    private var transport: URLSession!
    private var client: OllamaClient!

    override func setUp() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockOllama.self]
        transport = URLSession(configuration: config)
        client = OllamaClient(session: transport)
        MockOllama.respond = { _ in (200, #"{"models":[]}"#) }
        MockOllama.manage = { _ in (200, #"{"done":true,"done_reason":"unload"}"#) }
    }

    override func tearDown() async throws { transport.invalidateAndCancel() }

    func testPreloadPinsOnceAndQuitUnloads() async throws {
        var requests: [Int] = []
        MockOllama.manage = { request in
            let body = try MockOllama.payload(request)
            XCTAssertEqual(body["model"] as? String, "orca")
            XCTAssertEqual(body["stream"] as? Bool, false)
            requests.append(try XCTUnwrap(body["keep_alive"] as? Int))
            return (200, #"{"model":"orca:latest","done":true,"done_reason":"unload"}"#)
        }
        let session = ModelSession(client: client)
        let load = session.prepare("orca")
        XCTAssertTrue(session.isLoading)
        XCTAssertNotNil(session.loadingStarted)
        let loaded = await load.value
        XCTAssertTrue(loaded)
        XCTAssertEqual(session.activeModel, "orca")
        XCTAssertFalse(session.isLoading)
        XCTAssertNil(session.loadingStarted)
        _ = await session.prepare("orca").value
        let failures = await session.close(unload: true)
        XCTAssertTrue(failures.isEmpty)
        XCTAssertTrue(session.modelsToRelease.isEmpty)
        XCTAssertEqual(requests, [-1, 0])
    }

    func testSwitchLoadsReplacementBeforeConfirmedUnload() async throws {
        var actions: [String] = []
        MockOllama.manage = { request in
            let body = try MockOllama.payload(request)
            let model = try XCTUnwrap(body["model"] as? String)
            let keep = try XCTUnwrap(body["keep_alive"] as? Int)
            actions.append("\(model):\(keep)")
            return (200, #"{"done":true,"done_reason":"unload"}"#)
        }
        let session = ModelSession(client: client)
        session.approvePreviousUnload = { model in
            XCTAssertEqual(model, "first")
            XCTAssertEqual(session.activeModel, "second")
            return true
        }
        _ = await session.prepare("first").value
        _ = await session.prepare("second").value
        XCTAssertEqual(actions, ["first:-1", "second:-1", "first:0"])
        XCTAssertEqual(session.modelsToRelease, ["second"])
        _ = await session.close(unload: false)
    }

    func testDecliningUnloadKeepsModelsForOtherClients() async throws {
        var retention: [Int] = []
        MockOllama.manage = { request in
            retention.append(try XCTUnwrap(MockOllama.payload(request)["keep_alive"] as? Int))
            return (200, #"{"done":true}"#)
        }
        let session = ModelSession(client: client)
        session.approvePreviousUnload = { _ in false }
        _ = await session.prepare("first").value
        _ = await session.prepare("second").value
        let failures = await session.close(unload: false)
        XCTAssertTrue(failures.isEmpty)
        XCTAssertEqual(retention, [-1, -1])
    }

    func testLoadAndUnloadFailuresLeaveNoBusyState() async {
        MockOllama.manage = { _ in throw URLError(.timedOut) }
        let session = ModelSession(client: client)
        let loaded = await session.prepare("orca").value
        XCTAssertFalse(loaded)
        XCTAssertFalse(session.isLoading)
        XCTAssertNil(session.activeModel)
        XCTAssertNotNil(session.error)
        let failures = await session.close(unload: true)
        XCTAssertEqual(failures, ["orca"])
    }

    func testUnloadNeedsServerConfirmation() async throws {
        MockOllama.manage = { _ in (200, #"{"done":true,"done_reason":"load"}"#) }
        MockOllama.respond = { _ in (200, #"{"models":[{"name":"orca:latest","context_length":4096}]}"#) }
        do {
            try await client.setKeepAlive(model: "orca", seconds: 0)
            XCTFail("A still-loaded model must not count as unloaded")
        } catch { XCTAssertTrue(error.localizedDescription.contains("did not confirm unloading")) }
        MockOllama.respond = { _ in (200, #"{"models":[]}"#) }
        try await client.setKeepAlive(model: "orca", seconds: 0)
    }

    func testQuitDuringPreloadCannotMarkSessionReadyLater() async {
        let session = ModelSession(client: client)
        let load = session.prepare("orca")
        _ = await session.close(unload: false)
        let loaded = await load.value
        XCTAssertFalse(loaded)
        XCTAssertFalse(session.isLoading)
        XCTAssertNil(session.activeModel)
        let afterClosing = await session.prepare("orca").value
        XCTAssertFalse(afterClosing)
    }
}
