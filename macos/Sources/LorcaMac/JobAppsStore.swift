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
    let reply_revision: String?
    var provider_message: String? = nil
    var needsTemplateFill: Bool { body.range(of: #"\[[A-Z][^\]\n]*\](?!\()|_{3,}"#, options: .regularExpression) != nil }
}
struct JobMailSnapshot: Decodable, Equatable {
    struct ThreadMessage: Decodable, Identifiable, Equatable { let id: String; let from: String; let body: String; let sent: Bool; let date: String }
    struct Meta: Decodable, Equatable { let job_id: Int; let follow_up: String; let reviewed_reply: String? }
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

@MainActor
final class JobAppsStore: ObservableObject {
    enum AttentionStep: String {
        case application = "Complete application"
        case outreach = "Complete outreach"
        case reply = "Review reply"
        case followUp = "Review follow-up"
        case reconcile = "Check email status"
    }
    struct EmailEdit { let subject: String; let body: String; let token: String }
    @Published var emailEdits: [String: EmailEdit] = [:]
    @Published private(set) var snapshot: JobSnapshot?
    @Published private(set) var busy = false
    @Published var error: String?
    @Published var selected: Int?
    @Published private(set) var queueActive = false
    @Published private(set) var queueSkipped = Set<Int>()
    @Published private(set) var history: [String] = []
    @Published private(set) var preparingOutreach: Int?
    var openGmail: (URL) -> Void = { NSWorkspace.shared.open($0) }
    private let bridge: any JobAppsTransport
    private var poll: Task<Void, Never>?
    private var closing = false
    private var pendingRequests = 0
    private var pendingActions = 0
    private var errorOperation: String?
    init(bridge: any JobAppsTransport = JobAppsBridge()) { self.bridge = bridge }
    var running: Bool { snapshot?.run?.active == true }
    var selectedJob: JobRecord? { snapshot?.jobs.first { $0.id == selected } }
    var queueJobs: [JobRecord] {
        (snapshot?.jobs ?? []).filter { needsAttention($0) && !queueSkipped.contains($0.id) }
            .sorted { $0.displayNumber == $1.displayNumber ? $0.id < $1.id : $0.displayNumber < $1.displayNumber }
    }
    var attentionCount: Int { (snapshot?.jobs ?? []).filter { needsAttention($0) }.count }
    func beginQueue() {
        queueSkipped.removeAll();queueActive = true;selected = queueJobs.first?.id
    }
    func endQueue() {
        queueActive = false;queueSkipped.removeAll()
        if selected == nil { selected = snapshot?.jobs.max(by: { $0.displayNumber < $1.displayNumber })?.id }
    }
    func skipQueueJob() {
        guard queueActive, let selected else { return }
        queueSkipped.insert(selected);self.selected = queueJobs.first?.id
    }
    func nextQueueJob() { selected = queueJobs.first?.id }
    private func reconcileQueue() {
        guard queueActive else { return }
        // Keep the completed job visible until Next; polling must not move focus.
        if selectedJob == nil {
            let next = queueJobs.first?.id
            if selected != next { selected = next }
        }
    }
    func replyReceipt(_ id: Int) -> String {
        (snapshot?.mail?.messages ?? []).filter { $0.job_id == id && $0.state == "Needs review" }
            .map { $0.kind + ":" + (($0.reply_revision?.isEmpty == false ? $0.reply_revision : $0.sent_at) ?? "") }.sorted().joined(separator: "|")
    }
    func attentionStep(_ job: JobRecord, now: Date = Date()) -> AttentionStep? {
        guard !["Archived", "Rejected", "Offer"].contains(job.stage), job.verification != "Expired" else { return nil }
        // Applications and outreach are independent. Sending first never hides an unapplied job.
        if !["Applied", "Interviewing"].contains(job.stage) { return .application }
        let mail = snapshot?.mail
        let records = mail?.messages.filter { $0.job_id == job.id } ?? []
        let receipt = replyReceipt(job.id)
        let meta = mail?.meta.first { $0.job_id == job.id }
        if !receipt.isEmpty && receipt != (meta?.reviewed_reply ?? "") { return .reply }
        if records.contains(where: { ["Creating", "Sending", "Needs reconciliation", "Needs send reconciliation"].contains($0.state) }) { return .reconcile }
        if let due = meta?.follow_up, !due.isEmpty {
            let formatter = DateFormatter();formatter.dateFormat = "yyyy-MM-dd";formatter.timeZone = TimeZone(identifier: "America/New_York")
            if due <= formatter.string(from: now) { return .followUp }
        }
        if records.contains(where: { $0.kind == "initial" && ["Sent", "Needs review"].contains($0.state) && !($0.sent_at ?? "").isEmpty }) { return nil }
        return .outreach
    }
    func needsAttention(_ job: JobRecord) -> Bool {
        attentionStep(job) != nil
    }
    func actionLabel(_ job: JobRecord) -> String {
        if ["Archived", "Rejected", "Offer"].contains(job.stage) { return job.stage }
        if job.verification == "Expired" { return "Posting expired" }
        if attentionStep(job) == .application { return "Complete application" }
        if job.outreach_stage == "Replied" || snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Needs review" }) == true { return "Reply received" }
        if let due = snapshot?.mail?.meta.first(where: { $0.job_id == job.id })?.follow_up, !due.isEmpty {
            let format = DateFormatter();format.dateFormat = "yyyy-MM-dd";format.timeZone = TimeZone(identifier: "America/New_York")
            if due <= format.string(from: Date()) { return "Follow-up due" }
        }
        if snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Sent" && !($0.sent_at ?? "").isEmpty }) == true { return "Waiting for reply" }
        if snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && ["Creating", "Sending", "Needs reconciliation", "Needs send reconciliation"].contains($0.state) }) == true { return "Check email status" }
        if snapshot?.mail?.messages.contains(where: { $0.job_id == job.id && $0.state == "Gmail draft" }) == true { return "Review in Gmail" }
        if !job.recruiter_contact.contains("@") { return job.recruiter_name.isEmpty ? "Add recruiter" : "Add recruiter email" }
        return "Review & send"
    }
    func actionColor(_ job: JobRecord) -> Color {
        switch actionLabel(job) {
        case "Add recruiter", "Add recruiter email", "Follow-up due": return .yellow
        case "Reply received": return .blue
        case "Waiting for reply": return .green
        default: return .secondary
        }
    }
    func workflowStep(_ job: JobRecord) -> Int {
        if attentionStep(job) == .application { return 0 }
        if attentionStep(job) == nil { return 3 }
        if !job.recruiter_contact.contains("@"), snapshot?.mail?.messages.contains(where: { $0.job_id == job.id }) != true { return 1 }
        return 2
    }
    func waitingSummary(_ job: JobRecord) -> String {
        guard actionLabel(job) == "Waiting for reply" else { return actionLabel(job) }
        let due = snapshot?.mail?.meta.first { $0.job_id == job.id }?.follow_up ?? ""
        return "Waiting for response" + (due.isEmpty ? "" : " · Follow-up due " + due)
    }
    static func gmailDraftURL(_ message: JobMailRecord) -> URL {
        var url = URLComponents(string: "https://mail.google.com/mail/u/?authuser=lalithuriti%40gmail.com")!
        // Use Gmail's message ID, not the API's separate draft container ID.
        let id = message.provider_message ?? ""
        url.fragment = id.isEmpty ? "drafts" : "drafts?compose=" + id
        return url.url!
    }
    @discardableResult func prepareOutreach(_ job: Int, kind: String = "initial", fillTemplate: Bool = false) async -> Bool {
        guard preparingOutreach == nil, !busy, snapshot?.integrations?.busy != true else { return false }
        preparingOutreach = job
        defer { preparingOutreach = nil }
        guard await perform(["op":"prepare_outreach", "id":job, "kind":kind, "fillTemplate":fillTemplate]) else { return false }
        let deadline = Date().addingTimeInterval(180)
        while snapshot?.integrations?.busy == true {
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return false }
            guard Date() < deadline else { error = "Gmail is still working. Refresh its status before retrying; no email was sent by lorca."; return false }
            guard await perform(["op":"snapshot"]) else { return false }
        }
        guard let message = snapshot?.mail?.messages.first(where: { $0.job_id == job && $0.kind == kind }), message.state == "Gmail draft" else {
            error = snapshot?.integrations?.message ?? "Couldn’t confirm the Gmail draft. Refresh its status before retrying."
            return false
        }
        if fillTemplate && message.needsTemplateFill {
            error = snapshot?.integrations?.message ?? "The draft could not be filled. Your Gmail draft was left untouched."
            return false
        }
        dismissError()
        openGmail(Self.gmailDraftURL(message))
        return true
    }
    func open() async {
        observe()
        await perform(["op": "google_auto_sync"])
    }
    func observe() {
        guard !closing, poll == nil else { return }
        poll = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if self.pendingRequests == 0 { await self.perform(["op": "snapshot"]) }
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }
    @discardableResult func perform(_ fields: [String: Any]) async -> Bool {
        guard !closing else { return false }
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
                if fields["id"] as? Int == selected, self.history != history { self.history = history }
            } else if let snapshot = try? JSONDecoder().decode(JobSnapshot.self, from: result) {
                if self.snapshot != snapshot { self.snapshot = snapshot }
                if selected == nil && !queueActive { selected = snapshot.jobs.first?.id }
                reconcileQueue()
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
    func shutdown(timeout: TimeInterval = 35) async {
        guard !closing else { return }
        closing = true
        let started = poll != nil || snapshot != nil || busy
        poll?.cancel(); poll = nil
        if started {
            // Google writes finish one in-flight request; no fresh sync/network
            // work starts once the companion receives quit. Local state remains
            // durable even when offline, and sync resumes on the next open.
            _ = await finishWithin(timeout) { [bridge] in
                let data = try? JSONSerialization.data(withJSONObject: ["op": "quit"])
                if let data { _ = try? await bridge.request(data) }
            }
        }
        _ = await finishWithin(3) { [bridge] in await bridge.close() }
    }
}
