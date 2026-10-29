import XCTest
@testable import LorcaMac

@MainActor
private final class StalledBackend: MockBackend {
    var holdDraft = false
    var holdClose = false
    var waiting: CheckedContinuation<Void, Never>?
    var cancellations = 0
    override func command(_ fields: [String: Any]) async throws -> Data {
        let op = fields["op"] as? String
        if (op == "draft" && holdDraft) || (op == "close" && holdClose) {
            await withCheckedContinuation { waiting = $0 }
            throw BackendError(message: "Stopped stalled request")
        }
        return try await super.command(fields)
    }
    func release() { waiting?.resume(); waiting = nil }
    override func cancelRequests() async { cancellations += 1 }
}

private actor QuitTransport: JobAppsTransport {
    var waiting: CheckedContinuation<Void, Never>?
    var closed = false
    func request(_ payload: Data) async throws -> Data {
        let op = try JSONSerialization.jsonObject(with: payload) as! [String: Any]
        if op["op"] as? String == "quit" { await withCheckedContinuation { waiting = $0 } }
        return Data(#"{"ok":true,"result":{}}"#.utf8)
    }
    func close() { closed = true; waiting?.resume(); waiting = nil }
    func didClose() -> Bool { closed }
}

@MainActor
final class ShutdownTests: XCTestCase {
    private func makeStore(_ backend: MockBackend) -> ChatStore {
        let name = "lorca.shutdown-tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return ChatStore(backend: backend, defaults: defaults)
    }
    func testAppShutdownWaitsForCleanupAndRunsOnlyOnce() async {
        let shutdown = AppShutdown()
        var resume: CheckedContinuation<[String], Never>?
        var cleanupCalls = 0
        var completionCalls = 0
        shutdown.begin(unload: false, cleanup: { unload in
            XCTAssertFalse(unload)
            cleanupCalls += 1
            return await withCheckedContinuation { resume = $0 }
        }, completed: { failures in
            XCTAssertEqual(shutdown.phase, .ready)
            XCTAssertEqual(failures, ["unconfirmed"])
            completionCalls += 1
        })
        XCTAssertEqual(shutdown.phase, .closing)
        for _ in 0..<100 { if resume != nil { break }; await Task.yield() }
        shutdown.begin(unload: true, cleanup: { _ in XCTFail("Repeated shutdown"); return [] }, completed: { _ in XCTFail("Repeated completion") })
        XCTAssertEqual(cleanupCalls, 1)
        XCTAssertEqual(completionCalls, 0)
        resume?.resume(returning: ["unconfirmed"])
        for _ in 0..<100 { if completionCalls == 1 { break }; await Task.yield() }
        XCTAssertEqual(shutdown.phase, .ready)
        XCTAssertEqual(completionCalls, 1)
        shutdown.begin(unload: false, cleanup: { _ in XCTFail("Restarted shutdown"); return [] }, completed: { _ in XCTFail("Repeated completion") })
        XCTAssertEqual(cleanupCalls, 1)
    }

    func testDeadlineDoesNotWaitForUncooperativeCancellation() async {
        var blocked: CheckedContinuation<Int, Never>?
        let started = Date()
        let result = await finishWithin(0.03) { await withCheckedContinuation { blocked = $0 } }
        XCTAssertNil(result)
        XCTAssertLessThan(Date().timeIntervalSince(started), 1)
        blocked?.resume(returning: 7)
        await Task.yield() // Late completion must not resume the caller twice.
    }

    func testNormalQuitFlushesDraftAndClosesExactlyOnce() async throws {
        let backend = StalledBackend()
        defer { try? FileManager.default.removeItem(at: backend.root) }
        let store = makeStore(backend)
        await store.refreshConnection()
        store.draft = "Unsent but saved"
        let result = await store.modelSession.close(unload: false)
        XCTAssertTrue(result.isEmpty)
        XCTAssertEqual(backend.chats[0].draft, "Unsent but saved")
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "close" }.count, 1)
        XCTAssertFalse(store.canSend)
        store.newChat()
        _ = await store.modelSession.close(unload: false)
        XCTAssertEqual(backend.requests.filter { $0["op"] as? String == "close" }.count, 1)
        XCTAssertEqual(backend.cancellations, 2)
    }

    func testStalledDraftDoesNotBlockQuitAndRecoversNextLaunch() async throws {
        let backend = StalledBackend()
        defer { try? FileManager.default.removeItem(at: backend.root) }
        let store = makeStore(backend)
        await store.refreshConnection()
        backend.holdDraft = true
        store.draft = "Recover this exact draft 🫍"
        for _ in 0..<100 { if backend.waiting != nil { break }; await Task.yield() }
        XCTAssertNotNil(backend.waiting)
        let result = await store.shutdown(unload: false, drainSeconds: 0.03, closeSeconds: 0.1)
        XCTAssertTrue(result.isEmpty)
        let recovered = ConversationLibrary()
        recovered.recoverDrafts(at: backend.root.appendingPathComponent("pending-drafts.json"))
        recovered.apply(try backend.snapshot(), selected: backend.selected)
        XCTAssertEqual(recovered.current.draft, "Recover this exact draft 🫍")
        var restored: String?
        recovered.perform = { restored = $0["text"] as? String }
        recovered.restorePendingDrafts()
        XCTAssertEqual(restored, "Recover this exact draft 🫍")
        recovered.acknowledgedDraft(restored!, id: backend.selected)
        XCTAssertFalse(FileManager.default.fileExists(atPath: backend.root.appendingPathComponent("pending-drafts.json").path))
        backend.release()
    }

    func testStalledCloseReportsUnconfirmedUnloadAndReturns() async throws {
        let backend = StalledBackend()
        let store = makeStore(backend)
        await store.refreshConnection()
        backend.holdClose = true
        let result = await store.shutdown(unload: true, drainSeconds: 0.03, closeSeconds: 0.03)
        XCTAssertEqual(result, ["qwen3.5:9b"])
        backend.release()
    }

    func testStalledJobQuitClosesTransportAndPreventsNewWork() async throws {
        let bridge = QuitTransport()
        let jobs = JobAppsStore(bridge: bridge)
        _ = await jobs.perform(["op":"snapshot"])
        // Mark the transport as used even though the mock has no full snapshot.
        jobs.observe()
        await jobs.shutdown(timeout: 0.03)
        let closed = await bridge.didClose()
        XCTAssertTrue(closed)
        let submitted = await jobs.perform(["op":"google_auto_sync"])
        XCTAssertFalse(submitted)
    }

    func testStuckRealCompanionPipeCanBeClosedOutsideItsActor() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let script = root.appendingPathComponent("stalled.py")
        let ready = root.appendingPathComponent("ready")
        try "import pathlib,time,sys\npathlib.Path(\"\(ready.path)\").write_text('ready')\nfor line in sys.stdin: time.sleep(600)\n".write(to: script, atomically: true, encoding: .utf8)
        let bridge = JobAppsBridge(serviceURL: script)
        let request = Task { try? await bridge.request(Data(#"{"op":"snapshot"}"#.utf8)) }
        for _ in 0..<100 {
            if FileManager.default.fileExists(atPath: ready.path) { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: ready.path))
        let started = Date()
        await bridge.close()
        XCTAssertLessThan(Date().timeIntervalSince(started), 3)
        let result = await request.value
        XCTAssertNil(result)
        do { _ = try await bridge.request(Data()); XCTFail("Closed companion restarted") } catch { }
    }
}
