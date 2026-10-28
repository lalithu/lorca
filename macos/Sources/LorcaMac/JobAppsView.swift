import AppKit
import SwiftUI

private enum JobMode: String, CaseIterable { case research = "Research", applying = "Applying", email = "Email & Follow-ups" }
struct JobAppsView: View {
    private let zoom: CGFloat = 0 // Baseline metrics; the shared AppKit host scales the whole interface.
    @ObservedObject var store: JobAppsStore
    @ObservedObject var chat: ChatStore
    @State private var mode: JobMode = .research
    @State private var query = ""
    @State private var filter = "All"
    @State private var settings = false
    @State private var confirmingUnload = false
    private var modelToUnload: String? {
        store.snapshot?.run?.model ?? chat.modelSession.activeModel
    }
    private var jobs: [JobRecord] {
        if store.queueActive { return store.queueJobs }
        return (store.snapshot?.jobs ?? []).sorted { $0.displayNumber > $1.displayNumber }.filter { job in
            (query.isEmpty || "\(job.company) \(job.title) \(job.location)".localizedCaseInsensitiveContains(query))
                && (filter == "Follow-up due" && followUpDue(job.id) || filter == "All" || filter == "Verified" && job.verification == "Verified" || filter == "Needs review" && job.stage == "Ready for review" || filter == "With contact" && job.recruiter_contact.contains("@"))
        }
    }
    private func followUpDue(_ id: Int) -> Bool {
        guard let due = store.snapshot?.mail?.meta.first(where: { $0.job_id == id })?.follow_up, !due.isEmpty else { return false }
        let formatter = DateFormatter(); formatter.dateFormat = "yyyy-MM-dd"; formatter.timeZone = TimeZone(identifier: "America/New_York")
        return due <= formatter.string(from: Date())
    }
    var body: some View {
        VStack(spacing: 14) {
            ViewThatFits(in: .horizontal) {
                HStack { modePicker;Spacer();headerActions.fixedSize() }
                VStack(alignment: .leading, spacing: 10) { modePicker;HStack { Spacer();headerActions } }
            }
            if let run = store.snapshot?.run {
                HStack(spacing: 8) {
                    Circle().fill(run.active ? Color.green : Color.secondary).frame(width: 6, height: 6)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(run.active || run.status == "failed" ? run.message : run.status == "interrupted" ? "Research interrupted · Resume from saved progress" : "Research paused · Saved progress retained").font(.system(size: max(10, 12 + zoom))).lineLimit(2)
                        Text("\(run.model) · \(run.saved) saved this run · \(run.attempts) sessions").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                    }
                    Spacer()
                    researchControl
                }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
            } else {
                HStack {
                    Text("Source-backed opportunities, one role at a time.").font(.system(size: max(10, 12 + zoom))).foregroundStyle(.secondary)
                    Spacer(); researchControl
                }
            }
            if let google = store.snapshot?.integrations, google.connected {
                HStack {
                    Text(google.message).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary).lineLimit(2)
                    Spacer()
                    if google.busy { ProgressView().controlSize(.mini) }
                    else { Button("Sync now") { Task { await store.perform(["op":"google_sync"]) } }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small) }
                }
            }
            if let conflicts = store.snapshot?.mail?.conflicts, !conflicts.isEmpty {
                DisclosureGroup("\(conflicts.count) Sheet changes need review") {
                    ScrollView { VStack(alignment: .leading, spacing: 10) {
                        ForEach(conflicts) { conflict in
                            VStack(alignment: .leading, spacing: 5) {
                                Text("Job #\(conflict.job_id) · \(conflict.field)").font(.system(size: max(10, 11 + zoom))).bold()
                                Text("Lorca: \(conflict.local_value)\nSheet: \(conflict.remote_value)").font(.system(size: max(10, 11 + zoom))).textSelection(.enabled)
                                HStack {
                                    ForEach(["local","sheet"], id: \.self) { choice in
                                        Button(choice == "local" ? "Keep Lorca" : "Use Sheet") { Task { await store.perform(["op":"resolve_conflict","id":conflict.job_id,"field":conflict.field,"choice":choice]) } }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                                    }
                                }
                            }
                        }
                    } }.frame(maxHeight: 150)
                }.font(.system(size: max(10, 11 + zoom))).foregroundStyle(.orange)
            }
            JobAppsErrorView(store: store)
            if store.queueActive {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(store.selectedJob.map { store.needsAttention($0) ? store.attentionStep($0)!.rawValue : store.waitingSummary($0) } ?? (store.queueSkipped.isEmpty ? "You're caught up." : "You've reached the end of this pass."))
                            .font(.system(size: max(10, 13 + zoom), weight: .medium))
                        Text("Oldest first · \(store.queueJobs.count) remaining" + (store.queueSkipped.isEmpty ? "" : " · \(store.queueSkipped.count) skipped"))
                            .font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if let job = store.selectedJob {
                        if store.needsAttention(job) {
                            Button("Skip for now") { store.skipQueueJob() }.disabled(store.busy || store.preparingOutreach != nil)
                        } else {
                            Button("Next job →") { store.nextQueueJob() }.buttonStyle(.borderedProminent)
                        }
                    } else if !store.queueSkipped.isEmpty {
                        Button("Revisit skipped") { store.beginQueue() }
                    }
                    Button("Show all jobs") { store.endQueue() }
                }
            } else {
                HStack {
                    TextField("Search companies, roles, locations", text: $query).textFieldStyle(.roundedBorder)
                    Picker("Show", selection: $filter) { ForEach(["All", "Verified", "Needs review", "With contact", "Follow-up due"], id: \.self) { Text($0) } }.frame(width: 165)
                    Text("\(jobs.count)").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary).monospacedDigit()
                }
            }
            if store.snapshot?.jobs.isEmpty != false {
                VStack(spacing: 13) {
                    Image(systemName: "briefcase").font(.system(size: max(10, 30 + zoom))).foregroundStyle(.secondary)
                    Text("Your next opportunity starts here").font(.system(size: max(10, 14 + zoom), weight: .medium))
                    Text("Add your candidate profile in Settings, then start research.")
                        .font(.system(size: max(10, 12 + zoom))).foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 340)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                HSplitView {
                    if !store.queueActive {
                        ScrollViewReader { proxy in
                            List(selection: $store.selected) {
                                ForEach(jobs) { job in
                                    VStack(alignment: .leading, spacing: 5) {
                                        Text("\(job.displayNumber). \(job.company)").font(.system(size: max(10, 13 + zoom), weight: .semibold))
                                        Text(job.title).font(.system(size: max(10, 12 + zoom))).lineLimit(2)
                                        Text(store.waitingSummary(job)).font(.system(size: max(10, 11 + zoom))).foregroundStyle(store.actionColor(job))
                                    }.padding(.vertical, 5).tag(job.id).id(job.id)
                                }
                            }.listStyle(.inset)
                                .onChange(of: store.selected) { _, id in
                                    if store.queueActive, let id { withAnimation(.easeInOut(duration: 0.18)) { proxy.scrollTo(id, anchor: .top) } }
                                }
                                .onChange(of: store.queueActive) { _, active in
                                    if active, let id = store.selected { proxy.scrollTo(id, anchor: .top) }
                                }
                        }.frame(minWidth: 185, idealWidth: 245, maxWidth: 310)
                    }
                    if let job = store.selectedJob { detail(job).id(job.id).frame(minWidth: 230, maxWidth: .infinity) }
                    else { Text(store.queueActive ? (store.queueSkipped.isEmpty ? "All current actions are handled." : "Skipped jobs are saved for later.") : "Select an opportunity").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity) }
                }.background(.primary.opacity(0.02), in: RoundedRectangle(cornerRadius: 10))
            }
            HStack {
                Text("Apply manually · Review and send emails in Gmail").font(.system(size: max(10, 10 + zoom))).foregroundStyle(.secondary)
                Spacer()
                if let value = store.snapshot?.integrations?.sheet_url,
                   let url = URL(string: value), url.scheme == "https", url.host == "docs.google.com" {
                    Link("Open Google Sheet ↗", destination: url).font(.system(size: max(10, 10 + zoom)))
                }
                ZStack {
                    if store.busy { ProgressView().controlSize(.mini) }
                }.frame(width: 12, height: 12)
            }
        }
        .environment(\.jobAppsZoom, zoom)
        .font(.system(size: max(10, 13 + zoom)))
        .controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
        .padding(.horizontal, 28).padding(.bottom, 22)
        .task { await store.open();routeQueueStep() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            // Returning from Gmail should pick up the send without a manual refresh.
            Task { await store.perform(["op":"google_auto_sync"]) }
        }
        .onChange(of: store.selected) { _, id in
            routeQueueStep()
            if let id { Task { await store.perform(["op":"detail", "id":id]) } }
        }
        .onChange(of: store.queueActive) { _, _ in routeQueueStep() }
        .onChange(of: store.selectedJob.flatMap { store.attentionStep($0) }) { _, _ in routeQueueStep() }
        .onChange(of: jobs.map(\.id)) { _, ids in
            if !store.queueActive, let selected = store.selected, !ids.contains(selected) { store.selected = ids.first }
        }
        .sheet(isPresented: $settings) {
            ZoomContent(zoom: chat.uiZoom) { JobAppsSettings(store: store).environment(\.jobAppsZoom, zoom) }
        }
        .confirmationDialog("Unload \(modelToUnload ?? chat.selectedModel)?", isPresented: $confirmingUnload) {
            Button("Unload model", role: .destructive) {
                Task { await chat.unloadModel(model: modelToUnload); if let error = chat.error { store.error = error } }
            }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("Ollama shares models with other apps. Unloading can interrupt another app using this model. Your chats are kept; the model loads again when you send a message. Other active lorca sessions prevent unloading.")
        }
    }
    private var researchControl: some View {
        Group {
            if store.running { Button("Pause") { store.pause() }.buttonStyle(.bordered) }
            else { Button(store.snapshot?.run == nil ? "Start research" : "Resume") { store.start(model: chat.selectedModel) }.buttonStyle(.borderedProminent).disabled(chat.agent.busy || chat.isGenerating || chat.isManagingModel || chat.modelSession.isLoading || !chat.ready) }
        }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small).disabled(store.busy)
    }
    private var modePicker: some View {
        Picker("Mode", selection: $mode) { ForEach(JobMode.allCases, id: \.self) { Text($0.rawValue).tag($0) } }
            .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 440 + max(0, zoom) * 12)
    }
    private var headerActions: some View {
        HStack {
            Button("Needs you · \(store.attentionCount)") {
                if store.queueActive { store.endQueue() }
                else { query = "";filter = "All";store.beginQueue() }
            }.buttonStyle(.borderedProminent).tint(store.queueActive ? .yellow : .secondary)
                .foregroundStyle(store.queueActive ? Color.black : Color.primary)
                .accessibilityValue(store.queueActive ? "Oldest-first queue active" : "Off")
            Button("Unload model…") { confirmingUnload = true }
                .disabled(!chat.canReloadModel || modelToUnload == nil || store.busy || store.running || store.snapshot?.resumes?.work.busy == true)
            Button { settings = true } label: { Image(systemName: "slider.horizontal.3") }
                .help("Job Apps settings").accessibilityLabel("Job Apps settings")
        }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
    }
    private func routeQueueStep() {
        guard store.queueActive, let job = store.selectedJob, let step = store.attentionStep(job) else { return }
        mode = step == .application ? .applying : .email
    }
    private func detail(_ job: JobRecord) -> some View {
        let dossier = store.snapshot?.dossiers.first { $0.job_id == job.id }
        return ScrollViewReader { proxy in
        ScrollView {
            VStack(alignment: .leading, spacing: 17) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(job.company).font(.system(size: max(10, 20 + zoom), weight: .semibold))
                    Text(job.title).font(.system(size: max(10, 14 + zoom)))
                    Text(job.location).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                }.id("queue-detail-top")
                if store.queueActive { JobWorkflowView(store: store, job: job) }
                HStack {
                    if let url = URL(string: job.job_url), ["https","http"].contains(url.scheme ?? "") { Link("Open posting ↗", destination: url).font(.system(size: max(10, 11 + zoom))) }
                    Spacer()
                    Menu(job.stage) { ForEach(store.snapshot?.stages ?? [], id: \.self) { value in Button(value) { store.update(job, field: "stage", value: value) } } }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                }
                if mode == .email {
                    JobEmailDetail(store: store, job: job)
                } else if mode == .applying {
                    JobApplicationView(store: store, job: job, model: chat.selectedModel)
                } else {
                    if mode == .applying { Text("Open the official posting, apply yourself, then update the stage above. Application automation is deferred.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary) }
                    section("Role", dossier?.payload["role"]["summary"].string ?? job.description)
                    section("Why it could fit", dossier?.payload["role"]["fit"].string ?? "")
                    section("Check before applying", dossier?.payload["role"]["gaps"].string ?? "")
                    if !job.salary.isEmpty { section("Published compensation", job.salary) }
                    JobContactView(store: store, job: job)
                    if let payload = dossier?.payload {
                        DisclosureGroup("Sources & verification") {
                            VStack(alignment: .leading, spacing: 12) {
                                ForEach(["posting_evidence","application_evidence","salary_evidence"], id: \.self) { key in
                                    if let url = payload[key]["url"].string, let link = URL(string: url), link.scheme == "https" {
                                        Link(key.replacingOccurrences(of: "_", with: " "), destination: link).font(.system(size: max(10, 11 + zoom)))
                                        Text(payload[key]["quote"].string ?? "").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary).textSelection(.enabled)
                                    }
                                }
                                Text(payload["warnings"].strings.joined(separator: "\n")).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.orange)
                            }.padding(.top, 8)
                        }.font(.system(size: max(10, 11 + zoom)))
                    }
                }
                JobNotesView(store: store, job: job)
                DisclosureGroup("History") { ForEach(Array(store.history.enumerated()), id: \.offset) { _, entry in Text(entry).font(.system(size: max(10, 10 + zoom))).foregroundStyle(.secondary).textSelection(.enabled) } }.font(.system(size: max(10, 11 + zoom)))
            }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
        }
        .onChange(of: mode) { _, _ in if store.queueActive { proxy.scrollTo("queue-detail-top", anchor: .top) } }
        }
    }
    @ViewBuilder private func section(_ title: String, _ text: String) -> some View {
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            VStack(alignment: .leading, spacing: 6) { Text(title).font(.system(size: max(10, 12 + zoom), weight: .semibold));Text(text).font(.system(size: max(10, 12 + zoom))).lineSpacing(3).textSelection(.enabled) }
        }
    }
}
