import AppKit
import Combine
import Foundation
import UniformTypeIdentifiers

struct JobRecord: Codable, Identifiable, Equatable {
    let id: Int
    let number: Int?
    var displayNumber: Int { number ?? id }
    let company: String
    let title: String
    let location: String
    let stage: String
    let priority: String
    let verification: String
    let job_url: String
    let notes: String
    let salary: String
    let description: String
    let recruiter_name: String
    let recruiter_contact: String
    let outreach_stage: String
    let email_subject: String
    let email_draft: String
}
struct JobDossier: Decodable { let job_id: Int; let researched_at: String; let payload: AgentJSON }
struct JobRun: Decodable {
    let status: String
    let message: String
    let model: String
    let saved: Int
    let attempts: Int
    let failures: Int
    var active: Bool { ["starting", "running", "stopping"].contains(status) }
}
struct JobGoogleState: Decodable {
    let message: String
    let busy: Bool
    let connected: Bool
    let enabled: Bool
    let last_sync: String
    let sheet_url: String
    let initial_template: String
    let followup_template: String
    let resume_name: String
}
struct JobMailRecord: Decodable, Identifiable {
    let id: String
    let job_id: Int
    let kind: String
    let recipient: String
    let subject: String
    let body: String
    let state: String
    let sent_at: String?
    let error: String?
}
struct JobMailSnapshot: Decodable {
    struct Meta: Decodable { let job_id: Int; let follow_up: String }
    struct Conflict: Decodable, Identifiable {
        let job_id: Int; let field: String; let local_value: String; let remote_value: String
        var id: String { "\(job_id)-\(field)" }
    }
    let messages: [JobMailRecord]
    let meta: [Meta]
    let conflicts: [Conflict]
}
struct JobSnapshot: Decodable {
    struct Preferences: Decodable { let text: String; let version: String }
    let jobs: [JobRecord]
    let dossiers: [JobDossier]
    let run: JobRun?
    let preferences: Preferences
    let profile: String
    let root: String
    let stages: [String]
    let priorities: [String]
    let integrations: JobGoogleState?
    let mail: JobMailSnapshot?
}

// The existing Python tracker is a small stdio companion, never a web dashboard.
// Blocking pipe I/O stays off the main actor. All requests are serialized here.
private actor JobAppsBridge {
    private var child: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var pending = Data()
    func request(_ payload: Data) throws -> Data {
        if child?.isRunning != true {
            let resources = Bundle.main.bundleURL.appendingPathComponent("Contents/Resources/jobapps")
            let script = ProcessInfo.processInfo.environment["LORCA_JOBS_SERVICE"].map { URL(fileURLWithPath: $0) } ?? resources.appendingPathComponent("service.py")
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
            try process.run(); child = process; input = send.fileHandleForWriting; output = receive.fileHandleForReading; pending = Data()
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
    func close() { try? input?.close(); input = nil; output = nil; child = nil }
}

@MainActor
final class JobAppsStore: ObservableObject {
    @Published private(set) var snapshot: JobSnapshot?
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var selected: Int?
    @Published private(set) var history: [String] = []
    private let bridge = JobAppsBridge()
    private var poll: Task<Void, Never>?
    private var pendingRequests = 0
    var running: Bool { snapshot?.run?.active == true }
    var selectedJob: JobRecord? { snapshot?.jobs.first { $0.id == selected } }
    func observe() {
        guard poll == nil else { return }
        poll = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if !self.busy { await self.perform(["op": "snapshot"]) }
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }
    @discardableResult func perform(_ fields: [String: Any]) async -> Bool {
        // The bridge serializes requests; never silently drop a click during polling.
        pendingRequests += 1; busy = true
        defer { pendingRequests -= 1; busy = pendingRequests > 0 }
        do {
            let data = try JSONSerialization.data(withJSONObject: fields)
            let response = try await bridge.request(data)
            let value = try JSONSerialization.jsonObject(with: response) as! [String: Any]
            guard value["ok"] as? Bool == true else { throw BackendError(message: value["error"] as? String ?? "Job Apps failed") }
            let result = try JSONSerialization.data(withJSONObject: value["result"] ?? [:])
            if fields["op"] as? String == "detail" {
                let detail = value["result"] as? [String: Any]
                history = (detail?["history"] as? [[String: Any]] ?? []).map { "\($0["happened_at"] ?? "") · \($0["action"] ?? "")" }
            } else if let snapshot = try? JSONDecoder().decode(JobSnapshot.self, from: result) {
                self.snapshot = snapshot
                if selected == nil { selected = snapshot.jobs.first?.id }
            }
            error = nil; return true
        } catch { self.error = friendlyError(error); return false }
    }
    func start(model: String) { Task { await perform(["op":"start", "model":model]) } }
    func pause() { Task { await perform(["op":"pause"]) } }
    func update(_ job: JobRecord, field: String, value: String) { Task { await perform(["op":"update", "id":job.id, "field":field, "value":value]) } }
    func importTracker() {
        let panel = NSOpenPanel(); panel.canChooseFiles = false; panel.canChooseDirectories = true
        panel.message = "Choose the existing m2max-job-pipeline folder. The original tracker stays intact."
        panel.directoryURL = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await perform(["op":"import", "path":url.path]) }
    }
    func connectGoogle() {
        let legacy = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/lalith-job-search/google_token.json")
        var path = ""
        if !FileManager.default.fileExists(atPath: legacy.path) {
            let panel = NSOpenPanel(); panel.allowedContentTypes = [.json]
            panel.message = "Select your Google Desktop OAuth client JSON. Credentials stay on this Mac."
            guard panel.runModal() == .OK, let url = panel.url else { return }
            path = url.path
        }
        Task { await perform(["op":"google_connect", "path":path]) }
    }
    func chooseResume() {
        let panel = NSOpenPanel(); panel.allowedContentTypes = [.pdf]
        panel.message = "Select the approved PDF to attach to initial outreach drafts."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await perform(["op":"resume_pdf","path":url.path]) }
    }
    func shutdown() async {
        guard poll != nil || snapshot != nil || busy else { return }
        poll?.cancel(); poll = nil
        // Serialize after any in-flight request; quitting never leaves research running.
        let data = try? JSONSerialization.data(withJSONObject: ["op":"quit"])
        if let data { _ = try? await bridge.request(data) }
        await bridge.close()
    }
}
