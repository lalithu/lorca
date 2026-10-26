import Combine
import Foundation
import XCTest
@testable import LorcaMac

private actor JobAppsTestTransport: JobAppsTransport {
    private var response: Data
    private var requests: [String] = []
    private var held = Set<String>()
    private var waiting: [String: CheckedContinuation<Void, Never>] = [:]
    private var failures = Set<String>()

    init(response: Data) { self.response = response }
    func hold(_ op: String) { held.insert(op) }
    func resume(_ op: String) { waiting.removeValue(forKey: op)?.resume() }
    func isWaiting(_ op: String) -> Bool { waiting[op] != nil }
    func count(_ op: String) -> Int { requests.filter { $0 == op }.count }
    func replace(_ response: Data) { self.response = response }
    func fail(_ op: String) { failures.insert(op) }
    func recover(_ op: String) { failures.remove(op) }
    func close() { }

    func request(_ payload: Data) async throws -> Data {
        let fields = try JSONSerialization.jsonObject(with: payload) as! [String: Any]
        let op = fields["op"] as! String
        requests.append(op)
        if held.remove(op) != nil {
            await withCheckedContinuation { waiting[op] = $0 }
        }
        if failures.contains(op) { throw BackendError(message: "Companion unavailable") }
        if op == "detail" {
            return Data(#"{"ok":true,"result":{"history":[{"happened_at":"2026-10-26","action":"Saved"}]}}"#.utf8)
        }
        if op == "quit" { return Data(#"{"ok":true,"result":{"closed":true}}"#.utf8) }
        return response
    }
}

private func jobResponse(notes: String = "Original notes") throws -> Data {
    var job: [String: Any] = ["id": 7, "number": 49]
    for key in ["company", "title", "location", "stage", "priority", "verification", "job_url", "notes", "salary", "description", "recruiter_name", "recruiter_contact", "outreach_stage", "email_subject", "email_draft"] {
        job[key] = ""
    }
    job["company"] = "Test company"; job["notes"] = notes
    return try JSONSerialization.data(withJSONObject: ["ok": true, "result": [
        "jobs": [job], "dossiers": [], "run": NSNull(),
        "preferences": ["text": "Test preferences", "version": "1"],
        "profile": "Test profile", "root": "/test-only", "stages": [], "priorities": [],
        "integrations": ["message": "Connected", "busy": false, "connected": true, "enabled": true,
                         "last_sync": "", "sheet_url": "https://docs.google.com/spreadsheets/d/test",
                         "initial_template": "", "followup_template": "", "resume_name": ""],
        "mail": ["messages": [], "meta": [], "conflicts": []]
    ]])
}

private func queueResponse(stages: [String] = ["Ready for review", "Ready for review"], mailState: String? = nil, replyRevision: String = "", reviewed: String = "", due: String = "") throws -> Data {
    var envelope = try JSONSerialization.jsonObject(with: jobResponse()) as! [String: Any]
    var result = envelope["result"] as! [String: Any]
    let template = (result["jobs"] as! [[String: Any]])[0]
    result["jobs"] = stages.enumerated().map { offset, stage -> [String: Any] in
        var job = template
        job["id"] = offset + 7;job["number"] = [49, 2, 17][offset];job["stage"] = stage
        return job
    }
    var messages: [[String: Any]] = []
    if let mailState {
        messages = [["id":"initial", "job_id":8, "kind":"initial", "recipient":"recruiter@example.test", "subject":"Role", "body":"Hello", "state":mailState,
                     "sent_at": ["Sent", "Needs review"].contains(mailState) ? "2026-10-01T12:00:00Z" : "", "reply_revision":replyRevision]]
    }
    result["mail"] = ["messages":messages,"meta":[["job_id":8,"follow_up":due,"reviewed_reply":reviewed]],"conflicts":[]]
    envelope["result"] = result
    return try JSONSerialization.data(withJSONObject: envelope)
}

@MainActor
final class JobAppsTests: XCTestCase {
    func testQueueUsesDiscoveryOrderAndSkipNeverCompletesAJob() async throws {
        let bridge = JobAppsTestTransport(response: try queueResponse(stages: ["Ready for review", "Ready for review", "Ready for review"]))
        let store = JobAppsStore(bridge: bridge);await store.perform(["op":"snapshot"])
        store.beginQueue()
        XCTAssertEqual(store.queueJobs.map(\.displayNumber), [2,17,49]);XCTAssertEqual(store.selected,8)
        store.skipQueueJob();XCTAssertEqual(store.selected,9)
        await store.perform(["op":"snapshot"])
        XCTAssertEqual(store.selected,9);XCTAssertEqual(store.snapshot?.jobs.first(where: { $0.id == 8 })?.stage,"Ready for review")
        store.endQueue();store.beginQueue();XCTAssertEqual(store.selected,8)
        await store.shutdown()
    }
    func testQueueKeepsApplicationAndOutreachIndependentAndWaitsForSendReceipt() async throws {
        let bridge = JobAppsTestTransport(response: try queueResponse(mailState:"Sent"))
        let store = JobAppsStore(bridge: bridge);await store.perform(["op":"snapshot"]);store.beginQueue()
        XCTAssertEqual(store.attentionStep(store.selectedJob!),.application)
        await bridge.replace(try queueResponse(stages:["Ready for review","Applied"],mailState:"Local draft"))
        await store.perform(["op":"snapshot"])
        XCTAssertEqual(store.selected,8);XCTAssertEqual(store.attentionStep(store.selectedJob!),.outreach)
        await bridge.replace(try queueResponse(stages:["Ready for review","Applied"],mailState:"Needs send reconciliation"))
        await store.perform(["op":"snapshot"])
        XCTAssertEqual(store.selected,8);XCTAssertEqual(store.attentionStep(store.selectedJob!),.reconcile)
        await bridge.replace(try queueResponse(stages:["Ready for review","Applied"],mailState:"Sent"))
        await store.perform(["op":"snapshot"])
        XCTAssertEqual(store.selected,7)
        await store.shutdown()
    }
    func testQueueFinishesWithoutLoopingAndNewRepliesReturnWithoutInterruptingCurrentJob() async throws {
        let bridge = JobAppsTestTransport(response: try queueResponse(stages:["Archived","Applied"],mailState:"Sent"))
        let store = JobAppsStore(bridge: bridge);await store.perform(["op":"snapshot"]);store.beginQueue()
        XCTAssertNil(store.selected);XCTAssertTrue(store.queueJobs.isEmpty)
        await store.perform(["op":"snapshot"]);XCTAssertNil(store.selected)
        await bridge.replace(try queueResponse(stages:["Ready for review","Applied"],mailState:"Needs review",replyRevision:"r1",reviewed:"initial:r1"))
        await store.perform(["op":"snapshot"]);XCTAssertEqual(store.selected,7)
        await bridge.replace(try queueResponse(stages:["Ready for review","Applied"],mailState:"Needs review",replyRevision:"r2",reviewed:"initial:r1"))
        await store.perform(["op":"snapshot"])
        XCTAssertEqual(store.selected,7);XCTAssertEqual(store.queueJobs.first?.id,8)
        store.skipQueueJob();XCTAssertEqual(store.selected,8);XCTAssertEqual(store.attentionStep(store.selectedJob!),.reply)
        await store.shutdown()
    }
    func testQueueFollowupDueReturnsAndFutureDueDateWaits() async throws {
        let bridge = JobAppsTestTransport(response: try queueResponse(stages:["Archived","Applied"],mailState:"Sent",due:"2020-01-01"))
        let store = JobAppsStore(bridge: bridge);await store.perform(["op":"snapshot"]);store.beginQueue()
        XCTAssertEqual(store.selected,8);XCTAssertEqual(store.attentionStep(store.selectedJob!),.followUp)
        await bridge.replace(try queueResponse(stages:["Archived","Applied"],mailState:"Sent",due:"2099-01-01"))
        await store.perform(["op":"snapshot"]);XCTAssertNil(store.selected)
        await store.shutdown()
    }
    private func waitUntil(_ condition: () async -> Bool) async throws {
        for _ in 0..<1000 {
            if await condition() { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Timed out waiting for the test transport")
        throw BackendError(message: "Test timed out")
    }

    func testRepeatedPollingLeavesIdleUISelectionAndHistoryUnchanged() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        _ = await store.perform(["op": "snapshot"])
        _ = await store.perform(["op": "detail", "id": 7])
        var invalidations = 0, busyStates: [Bool] = []
        let updates = store.objectWillChange.sink { invalidations += 1 }
        let busy = store.$busy.dropFirst().sink { busyStates.append($0) }
        store.observe()
        // Exercise the actual immediate poll and the next three-second poll.
        try await waitUntil { await bridge.count("snapshot") >= 3 }
        XCTAssertEqual(invalidations, 0)
        XCTAssertTrue(busyStates.isEmpty)
        XCTAssertEqual(store.selected, 7)
        XCTAssertEqual(store.history, ["2026-10-26 · Saved"])
        XCTAssertEqual(store.selectedJob?.notes, "Original notes")
        await store.shutdown()
        withExtendedLifetime((updates, busy)) { }
    }

    func testOpeningJobAppsRequestsSyncWithoutPollingOrBusyFlicker() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        var busyStates: [Bool] = []
        let busy = store.$busy.dropFirst().sink { busyStates.append($0) }
        await store.open()
        try await waitUntil { await bridge.count("snapshot") >= 2 }
        let firstCount = await bridge.count("google_auto_sync")
        XCTAssertEqual(firstCount, 1)
        XCTAssertTrue(busyStates.isEmpty)
        await store.open()
        let secondCount = await bridge.count("google_auto_sync")
        XCTAssertEqual(secondCount, 2)
        XCTAssertTrue(busyStates.isEmpty)
        await store.shutdown()
        withExtendedLifetime(busy) { }
    }

    func testResumeDropAcceptsOneLocalPDFAndPreservesValidationErrors() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        _ = await store.perform(["op": "snapshot"])
        let pdf = URL(fileURLWithPath: "/tmp/test-resume.PDF")
        XCTAssertFalse(store.importResume([URL(fileURLWithPath: "/tmp/resume.txt")]))
        XCTAssertFalse(store.importResume([pdf, pdf]))
        XCTAssertFalse(store.importResume([URL(string: "https://example.com/resume.pdf")!]))
        _ = await store.perform(["op": "snapshot"])
        XCTAssertEqual(store.error, "Choose or drop one PDF resume.")
        let invalidCount = await bridge.count("resume_pdf")
        XCTAssertEqual(invalidCount, 0)
        XCTAssertTrue(store.importResume([pdf]))
        try await waitUntil { await bridge.count("resume_pdf") == 1 }
        try await waitUntil { !store.busy }
        XCTAssertNil(store.error)
        await store.shutdown()
    }

    func testChangedSnapshotPublishesOnceWithoutTogglingBusy() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        _ = await store.perform(["op": "snapshot"])
        var invalidations = 0, busyStates: [Bool] = []
        let updates = store.objectWillChange.sink { invalidations += 1 }
        let busy = store.$busy.dropFirst().sink { busyStates.append($0) }
        await bridge.replace(try jobResponse(notes: "Updated remotely"))
        _ = await store.perform(["op": "snapshot"])
        _ = await store.perform(["op": "snapshot"])
        XCTAssertEqual(invalidations, 1)
        XCTAssertTrue(busyStates.isEmpty)
        XCTAssertEqual(store.selectedJob?.notes, "Updated remotely")
        XCTAssertEqual(store.selected, 7)
        await store.shutdown()
        withExtendedLifetime((updates, busy)) { }
    }

    func testActionRemainsBusyWhenBackgroundReadFinishes() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        _ = await store.perform(["op": "snapshot"])
        var busyStates: [Bool] = []
        let busy = store.$busy.dropFirst().sink { busyStates.append($0) }
        await bridge.hold("snapshot"); await bridge.hold("update")
        let poll = Task { await store.perform(["op": "snapshot"]) }
        try await waitUntil { await bridge.isWaiting("snapshot") }
        XCTAssertFalse(store.busy)
        let action = Task { await store.perform(["op": "update", "id": 7, "field": "notes", "value": "Saved"]) }
        try await waitUntil { await bridge.isWaiting("update") }
        XCTAssertTrue(store.busy)
        await bridge.resume("snapshot"); _ = await poll.value
        XCTAssertTrue(store.busy, "A read must not clear a pending action's loading state")
        await bridge.resume("update"); let saved = await action.value
        XCTAssertTrue(saved)
        XCTAssertFalse(store.busy)
        XCTAssertEqual(busyStates, [true, false])
        await store.shutdown()
        withExtendedLifetime(busy) { }
    }

    func testActionErrorSurvivesSuccessfulPollingUntilRetryOrDismissal() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        await bridge.fail("start")
        _ = await store.perform(["op": "start"])
        XCTAssertEqual(store.error, "Companion unavailable")
        _ = await store.perform(["op": "snapshot"])
        _ = await store.perform(["op": "detail", "id": 7])
        XCTAssertEqual(store.error, "Companion unavailable")
        await bridge.fail("snapshot")
        _ = await store.perform(["op": "snapshot"])
        await bridge.recover("snapshot")
        _ = await store.perform(["op": "snapshot"])
        XCTAssertEqual(store.error, "Companion unavailable", "A read must not take ownership of an action error")
        await bridge.recover("start")
        _ = await store.perform(["op": "start"])
        XCTAssertNil(store.error)
        await bridge.fail("start")
        _ = await store.perform(["op": "start"])
        store.dismissError()
        XCTAssertNil(store.error)
        _ = await store.perform(["op": "snapshot"])
        XCTAssertNil(store.error)
        await store.shutdown()
    }

    func testRepeatedErrorsDoNotInvalidateUIAndFailedActionClearsBusy() async throws {
        let bridge = JobAppsTestTransport(response: try jobResponse())
        let store = JobAppsStore(bridge: bridge)
        _ = await store.perform(["op": "snapshot"])
        await bridge.fail("update")
        let saved = await store.perform(["op": "update"])
        XCTAssertFalse(saved); XCTAssertFalse(store.busy)
        XCTAssertEqual(store.error, "Companion unavailable")
        var invalidations = 0
        let updates = store.objectWillChange.sink { invalidations += 1 }
        await bridge.fail("snapshot")
        _ = await store.perform(["op": "snapshot"])
        _ = await store.perform(["op": "snapshot"])
        XCTAssertEqual(invalidations, 0)
        XCTAssertEqual(store.selectedJob?.notes, "Original notes")
        await store.shutdown()
        withExtendedLifetime(updates) { }
    }
}
