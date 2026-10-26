import SwiftUI

private enum JobMode: String, CaseIterable { case research = "Research", applying = "Applying", email = "Email & Follow-ups" }
struct JobAppsView: View {
    @ObservedObject var store: JobAppsStore
    @ObservedObject var chat: ChatStore
    @State private var mode: JobMode = .research
    @State private var query = ""
    @State private var filter = "All"
    @State private var settings = false
    private var jobs: [JobRecord] {
        (store.snapshot?.jobs ?? []).sorted { $0.id > $1.id }.filter { job in
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
            HStack {
                Picker("Mode", selection: $mode) { ForEach(JobMode.allCases, id: \.self) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented).frame(maxWidth: 440)
                Spacer()
                Button { settings = true } label: { Image(systemName: "slider.horizontal.3") }.help("Job Apps settings")
            }
            if let run = store.snapshot?.run {
                HStack(spacing: 8) {
                    Circle().fill(run.active ? Color.green : Color.secondary).frame(width: 6, height: 6)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(run.active || run.status == "failed" ? run.message : run.status == "interrupted" ? "Research interrupted · Resume from saved progress" : "Research paused · Saved progress retained").font(.system(size: 12)).lineLimit(2)
                        Text("\(run.model) · \(run.saved) saved this run · \(run.attempts) sessions").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    researchControl
                }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
            } else {
                HStack {
                    Text("Source-backed opportunities, one role at a time.").font(.system(size: 12)).foregroundStyle(.secondary)
                    Spacer(); researchControl
                }
            }
            if let google = store.snapshot?.integrations, google.connected {
                HStack {
                    Text(google.message).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                    Spacer()
                    if google.busy { ProgressView().controlSize(.mini) }
                    else { Button("Sync now") { Task { await store.perform(["op":"google_sync"]) } }.controlSize(.small) }
                }
            }
            if let conflicts = store.snapshot?.mail?.conflicts, !conflicts.isEmpty {
                DisclosureGroup("\(conflicts.count) Sheet changes need review") {
                    ScrollView { VStack(alignment: .leading, spacing: 10) {
                        ForEach(conflicts) { conflict in
                            VStack(alignment: .leading, spacing: 5) {
                                Text("Job #\(conflict.job_id) · \(conflict.field)").font(.caption).bold()
                                Text("Lorca: \(conflict.local_value)\nSheet: \(conflict.remote_value)").font(.caption).textSelection(.enabled)
                                HStack {
                                    ForEach(["local","sheet"], id: \.self) { choice in
                                        Button(choice == "local" ? "Keep Lorca" : "Use Sheet") { Task { await store.perform(["op":"resolve_conflict","id":conflict.job_id,"field":conflict.field,"choice":choice]) } }.controlSize(.small)
                                    }
                                }
                            }
                        }
                    } }.frame(maxHeight: 150)
                }.font(.caption).foregroundStyle(.orange)
            }
            if let error = store.error { Text(error).font(.caption).foregroundStyle(.orange).frame(maxWidth: .infinity, alignment: .leading).textSelection(.enabled) }
            HStack {
                TextField("Search companies, roles, locations", text: $query).textFieldStyle(.roundedBorder)
                Picker("Show", selection: $filter) { ForEach(["All", "Verified", "Needs review", "With contact", "Follow-up due"], id: \.self) { Text($0) } }.frame(width: 165)
                Text("\(jobs.count)").font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            if store.snapshot?.jobs.isEmpty != false {
                VStack(spacing: 13) {
                    Image(systemName: "briefcase").font(.system(size: 30)).foregroundStyle(.secondary)
                    Text("Your next opportunity starts here").font(.headline)
                    Text("Import your saved tracker or add your candidate profile in Settings, then start research.")
                        .font(.system(size: 12)).foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 340)
                    Button("Import existing tracker") { store.importTracker() }.disabled(store.busy)
                }.frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                HSplitView {
                    List(selection: $store.selected) {
                        ForEach(jobs) { job in
                            VStack(alignment: .leading, spacing: 5) {
                                Text("\(job.id). \(job.company)").font(.system(size: 13, weight: .semibold))
                                Text(job.title).font(.system(size: 12)).lineLimit(2)
                                if mode == .email { Text(followUpDue(job.id) ? "Follow-up due" : job.outreach_stage).font(.caption).foregroundStyle(followUpDue(job.id) ? Color.orange : Color.secondary) }
                                HStack { Text(job.location).lineLimit(1); Spacer(); Text(job.verification).foregroundStyle(job.verification == "Verified" ? Color.green : Color.secondary) }
                                    .font(.system(size: 10)).foregroundStyle(.secondary)
                            }.padding(.vertical, 5).tag(job.id)
                        }
                    }.listStyle(.inset).frame(minWidth: 185, idealWidth: 245, maxWidth: 310)
                    if let job = store.selectedJob { detail(job).frame(minWidth: 230, maxWidth: .infinity) }
                    else { Text("Select an opportunity").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity) }
                }.background(.primary.opacity(0.02), in: RoundedRectangle(cornerRadius: 10))
            }
            HStack {
                Text("Exa free · Manual applications · Gmail drafts only").font(.system(size: 10)).foregroundStyle(.secondary)
                Spacer()
                if let value = store.snapshot?.integrations?.sheet_url,
                   let url = URL(string: value), url.scheme == "https", url.host == "docs.google.com" {
                    Link("Open Google Sheet ↗", destination: url).font(.system(size: 10))
                }
                if store.busy { ProgressView().controlSize(.mini) }
            }
        }
        .padding(.horizontal, 28).padding(.bottom, 22)
        .onAppear { store.observe() }
        .onChange(of: store.selected) { _, id in if let id { Task { await store.perform(["op":"detail", "id":id]) } } }
        .sheet(isPresented: $settings) { JobAppsSettings(store: store) }
    }
    private var researchControl: some View {
        Group {
            if store.running { Button("Pause") { store.pause() }.buttonStyle(.bordered) }
            else { Button(store.snapshot?.run == nil ? "Start research" : "Resume") { store.start(model: chat.selectedModel) }.buttonStyle(.borderedProminent).disabled(chat.agent.busy || chat.isGenerating || !chat.ready) }
        }.controlSize(.small).disabled(store.busy)
    }
    private func detail(_ job: JobRecord) -> some View {
        let dossier = store.snapshot?.dossiers.first { $0.job_id == job.id }
        return ScrollView {
            VStack(alignment: .leading, spacing: 17) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(job.company).font(.system(size: 20, weight: .semibold))
                    Text(job.title).font(.system(size: 14))
                    Text(job.location).font(.caption).foregroundStyle(.secondary)
                }
                HStack {
                    if let url = URL(string: job.job_url), ["https","http"].contains(url.scheme ?? "") { Link("Open posting ↗", destination: url).font(.caption) }
                    Spacer()
                    Menu(job.stage) { ForEach(store.snapshot?.stages ?? [], id: \.self) { value in Button(value) { store.update(job, field: "stage", value: value) } } }.controlSize(.small)
                }
                if mode == .email {
                    JobEmailDetail(store: store, job: job)
                } else {
                    if mode == .applying { Text("Open the official posting, apply yourself, then update the stage above. Application automation is deferred.").font(.caption).foregroundStyle(.secondary) }
                    section("Role", dossier?.payload["role"]["summary"].string ?? job.description)
                    section("Why it could fit", dossier?.payload["role"]["fit"].string ?? "")
                    section("Check before applying", dossier?.payload["role"]["gaps"].string ?? "")
                    if !job.salary.isEmpty { section("Published compensation", job.salary) }
                    section("Contact", job.recruiter_name + "\n" + job.recruiter_contact)
                    if let payload = dossier?.payload {
                        DisclosureGroup("Sources & verification") {
                            VStack(alignment: .leading, spacing: 12) {
                                ForEach(["posting_evidence","application_evidence","salary_evidence"], id: \.self) { key in
                                    if let url = payload[key]["url"].string, let link = URL(string: url), link.scheme == "https" {
                                        Link(key.replacingOccurrences(of: "_", with: " "), destination: link).font(.caption)
                                        Text(payload[key]["quote"].string ?? "").font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                                    }
                                }
                                Text(payload["warnings"].strings.joined(separator: "\n")).font(.caption).foregroundStyle(.orange)
                            }.padding(.top, 8)
                        }.font(.caption)
                    }
                }
                JobNotesView(store: store, job: job)
                DisclosureGroup("History") { ForEach(Array(store.history.enumerated()), id: \.offset) { _, entry in Text(entry).font(.system(size: 10)).foregroundStyle(.secondary).textSelection(.enabled) } }.font(.caption)
            }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
        }
    }
    @ViewBuilder private func section(_ title: String, _ text: String) -> some View {
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            VStack(alignment: .leading, spacing: 6) { Text(title).font(.system(size: 12, weight: .semibold));Text(text).font(.system(size: 12)).lineSpacing(3).textSelection(.enabled) }
        }
    }
}

private struct JobAppsSettings: View {
    @ObservedObject var store: JobAppsStore
    @Environment(\.dismiss) private var dismiss
    @State private var preferences = ""
    @State private var profile = ""
    @State private var version = ""
    @State private var section = "Search & profile"
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Job Apps settings").font(.title2)
            Picker("Section", selection: $section) {
                Text("Search & profile").tag("Search & profile")
                Text("Google & templates").tag("Google & templates")
            }.pickerStyle(.segmented)
            if section == "Google & templates" {
                JobGoogleSettings(store: store)
                HStack { Spacer(); Button("Done") { dismiss() } }
            } else {
            Text("Search preferences").font(.headline)
            TextEditor(text: $preferences).font(.system(size: 12)).frame(height: 150).border(.secondary.opacity(0.2))
            Text("Candidate profile · Facts only").font(.headline)
            TextEditor(text: $profile).font(.system(size: 12)).frame(height: 170).border(.secondary.opacity(0.2))
            Text("Changes apply to the next research session. Pause research before saving.").font(.caption).foregroundStyle(.secondary)
            if let error = store.error { Text(error).font(.caption).foregroundStyle(.orange) }
            HStack {
                Button("Import existing tracker") { store.importTracker() }.disabled(store.snapshot?.jobs.isEmpty == false || store.busy)
                Spacer();Button("Cancel") { dismiss() }
                Button("Save") { Task { if await store.perform(["op":"settings","preferences":preferences,"profile":profile,"version":version]) { dismiss() } } }
                    .buttonStyle(.borderedProminent).disabled(store.busy || store.running)
            }
            }
        }.padding(24).frame(width: 620)
        .onAppear { preferences = store.snapshot?.preferences.text ?? ""; profile = store.snapshot?.profile ?? ""; version = store.snapshot?.preferences.version ?? "" }
    }
}

private struct JobNotesView: View {
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    @State private var notes = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                Text("Notes").font(.system(size: 12, weight: .semibold))
                Spacer()
                Menu(job.priority + " priority") {
                    ForEach(store.snapshot?.priorities ?? [], id: \.self) { value in
                        Button(value) { store.update(job, field: "priority", value: value) }
                    }
                }.font(.caption).controlSize(.small)
            }
            TextEditor(text: $notes).font(.system(size: 12)).frame(minHeight: 70).border(.secondary.opacity(0.15))
            if notes != job.notes {
                Button("Save notes") { store.update(job, field: "notes", value: notes) }.controlSize(.small).disabled(store.busy)
            }
        }.onAppear { notes = job.notes }.onChange(of: job.id) { _, _ in notes = job.notes }
    }
}
