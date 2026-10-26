import AppKit
import Combine
import Foundation
import UniformTypeIdentifiers
import SwiftUI

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
struct JobDossier: Decodable, Equatable { let job_id: Int; let researched_at: String; let payload: AgentJSON }
struct JobRun: Decodable, Equatable {
    let status: String
    let message: String
    let model: String
    let saved: Int
    let attempts: Int
    let failures: Int
    var active: Bool { ["starting", "running", "stopping"].contains(status) }
}
struct JobSettingsConflict: Decodable, Identifiable, Equatable {
    let field: String
    let local_value: String
    let remote_value: String
    var id: String { field }
}
struct JobGoogleState: Decodable, Equatable {
    let message: String
    let busy: Bool
    let connected: Bool
    let enabled: Bool
    let last_sync: String
    let sheet_url: String
    let initial_template: String
    let followup_template: String
    let resume_name: String
    let latex_source: String?
    let resume_sha256: String?
    let resume_conflict: Bool?
    let settings_conflicts: [JobSettingsConflict]?
    let templates_version: String?
}
struct JobMailRecord: Decodable, Identifiable, Equatable {
    let id: String
    let job_id: Int
    let kind: String
    let recipient: String
    let subject: String
    let body: String
    let state: String
    let sent_at: String?
    let error: String?
    let approval_token: String?
}
struct JobMailSnapshot: Decodable, Equatable {
    struct ThreadMessage: Decodable, Identifiable, Equatable { let id: String; let from: String; let body: String; let sent: Bool; let date: String }
    struct Meta: Decodable, Equatable { let job_id: Int; let follow_up: String }
    struct Conflict: Decodable, Identifiable, Equatable {
        let job_id: Int; let field: String; let local_value: String; let remote_value: String
        var id: String { "\(job_id)-\(field)" }
    }
    let messages: [JobMailRecord]
    let meta: [Meta]
    let conflicts: [Conflict]
    let threads: [String: [ThreadMessage]]?
}
struct JobResumeSnapshot: Decodable, Equatable {
    struct Work: Decodable, Equatable { let busy: Bool; let job_id: Int?; let message: String }
    struct Record: Decodable, Equatable {
        let job_id: Int; let source: String; let master_source: String; let version: String
        let selection: String; let saved_version: String; let submitted_version: String; let pdf_path: String
    }
    let work: Work; let records: [Record]; let compiler: Bool
}
struct JobSnapshot: Decodable, Equatable {
    struct Preferences: Decodable, Equatable { let text: String; let version: String }
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
    let resumes: JobResumeSnapshot?
}

protocol JobAppsTransport: Sendable {
    func request(_ payload: Data) async throws -> Data
    func close() async
}

// The existing Python tracker is a small stdio companion, never a web dashboard.
// Blocking pipe I/O stays off the main actor. All requests are serialized here.
private actor JobAppsBridge: JobAppsTransport {
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
    struct EmailEdit { let subject: String; let body: String; let token: String }
    @Published var emailEdits: [String: EmailEdit] = [:]
    @Published private(set) var snapshot: JobSnapshot?
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var selected: Int?
    @Published private(set) var history: [String] = []
    private let bridge: any JobAppsTransport
    private var poll: Task<Void, Never>?
    private var pendingRequests = 0
    private var pendingActions = 0
    private var errorOperation: String?
    init(bridge: any JobAppsTransport = JobAppsBridge()) { self.bridge = bridge }
    var running: Bool { snapshot?.run?.active == true }
    var selectedJob: JobRecord? { snapshot?.jobs.first { $0.id == selected } }
    func needsAttention(_ job: JobRecord) -> Bool {
        !["Archived", "Rejected", "Offer"].contains(job.stage) && actionLabel(job) != "Waiting for reply"
    }
    func actionLabel(_ job: JobRecord) -> String {
        if job.outreach_stage == "Replied" || snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Needs review" }) == true { return "Reply received" }
        if let due = snapshot?.mail?.meta.first(where: { $0.job_id == job.id })?.follow_up, !due.isEmpty {
            let format = DateFormatter();format.dateFormat = "yyyy-MM-dd";format.timeZone = TimeZone(identifier: "America/New_York")
            if due <= format.string(from: Date()) { return "Follow-up due" }
        }
        if job.outreach_stage == "Sent" || snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Sent" }) == true { return "Waiting for reply" }
        if snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && ["Creating", "Sending", "Needs reconciliation", "Needs send reconciliation"].contains($0.state) }) == true { return "Check email status" }
        if snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Gmail draft" }) == true { return "Review in Gmail" }
        if !job.recruiter_contact.contains("@") { return job.recruiter_name.isEmpty ? "Add recruiter" : "Add recruiter email" }
        return "Review & send"
    }
    func actionColor(_ job: JobRecord) -> Color {
        switch actionLabel(job) {
        case "Add recruiter", "Add recruiter email", "Follow-up due": return .yellow
        case "Reply received": return .blue
        default: return .secondary
        }
    }
    func open() async {
        observe()
        await perform(["op": "google_auto_sync"])
    }
    func observe() {
        guard poll == nil else { return }
        poll = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if self.pendingRequests == 0 { await self.perform(["op": "snapshot"]) }
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }
    @discardableResult func perform(_ fields: [String: Any]) async -> Bool {
        // Reads are silent. User actions still queue behind a poll and retain
        // their busy state until every pending action has finished.
        let operation = fields["op"] as? String ?? ""
        let action = !["snapshot", "detail", "google_auto_sync"].contains(operation)
        pendingRequests += 1
        if action { pendingActions += 1; if !busy { busy = true } }
        defer {
            pendingRequests -= 1
            if action {
                pendingActions -= 1
                if busy != (pendingActions > 0) { busy = pendingActions > 0 }
            }
        }
        do {
            let data = try JSONSerialization.data(withJSONObject: fields)
            let response = try await bridge.request(data)
            let value = try JSONSerialization.jsonObject(with: response) as! [String: Any]
            guard value["ok"] as? Bool == true else { throw BackendError(message: value["error"] as? String ?? "Job Apps failed") }
            let result = try JSONSerialization.data(withJSONObject: value["result"] ?? [:])
            if fields["op"] as? String == "detail" {
                let detail = value["result"] as? [String: Any]
                let history = (detail?["history"] as? [[String: Any]] ?? []).map { "\($0["happened_at"] ?? "") · \($0["action"] ?? "")" }
                if self.history != history { self.history = history }
            } else if let snapshot = try? JSONDecoder().decode(JobSnapshot.self, from: result) {
                if self.snapshot != snapshot { self.snapshot = snapshot }
                if selected == nil { selected = snapshot.jobs.first?.id }
            }
            if error != nil, errorOperation == operation { dismissError() }
            return true
        } catch {
            let message = friendlyError(error)
            if action || self.error == nil || errorOperation == operation {
                if self.error != message { self.error = message }
                errorOperation = operation
            }
            return false
        }
    }
    func dismissError() {
        if error != nil { error = nil }
        errorOperation = nil
    }
    func start(model: String) { Task { await perform(["op":"start", "model":model]) } }
    func pause() { Task { await perform(["op":"pause"]) } }
    func update(_ job: JobRecord, field: String, value: String) { Task { await perform(["op":"update", "id":job.id, "field":field, "value":value]) } }
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
        importResume([url])
    }
    func chooseLatex() {
        let panel = NSOpenPanel();panel.allowedContentTypes = [.init(filenameExtension: "tex") ?? .plainText]
        panel.message = "Choose your master LaTeX resume. Source text syncs through your private Sheet."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            let data = try Data(contentsOf: url)
            guard data.count <= 24000, let text = String(data: data, encoding: .utf8) else { throw BackendError(message: "Choose a UTF-8 LaTeX source under 24 KB.") }
            Task { await perform(["op":"resume_source","source":text]) }
        } catch { self.error = friendlyError(error) }
    }
    @discardableResult func importResume(_ urls: [URL]) -> Bool {
        guard urls.count == 1, let url = urls.first, url.isFileURL, url.pathExtension.lowercased() == "pdf" else {
            error = "Choose or drop one PDF resume."; errorOperation = "resume_pdf"; return false
        }
        guard !busy, snapshot?.integrations?.busy != true else {
            error = "Wait for the current operation before replacing your resume."; errorOperation = "resume_pdf"; return false
        }
        Task {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            await perform(["op": "resume_pdf", "path": url.path])
        }
        return true
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
