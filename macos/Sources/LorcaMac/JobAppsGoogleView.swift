import SwiftUI

struct JobGoogleSettings: View {
    @ObservedObject var store: JobAppsStore
    @State private var sheet = ""
    @State private var initial = ""
    @State private var followup = ""
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 13) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("lalithuriti@gmail.com").font(.headline)
                        Text(store.snapshot?.integrations?.message ?? "Not connected").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Connect Google…") { store.connectGoogle() }.disabled(store.snapshot?.integrations?.busy == true)
                }
                Text("Allows creating drafts and reading linked Job Apps threads. Lorca never sends email. Google credentials stay local.").font(.caption).foregroundStyle(.secondary)
                TextField("Google Sheet URL", text: $sheet).textFieldStyle(.roundedBorder)
                HStack {
                    Button("Save connection & templates") { save() }
                    Button(store.snapshot?.integrations?.enabled == true ? "Sync now" : "Enable Sheets sync") {
                        Task {
                            if await store.perform(["op":"google_settings","sheet_url":sheet,"initial_template":initial,"followup_template":followup]) {
                                await store.perform(["op":"google_enable"])
                            }
                        }
                    }.disabled(store.snapshot?.integrations?.connected != true || store.snapshot?.integrations?.busy == true)
                }.controlSize(.small)
                Text("Syncs every minute while Lorca is open. Sheet edits to notes, priority, stage and follow-up date sync back; conflicting edits are kept for review. Enabling replaces the old sync service.").font(.caption).foregroundStyle(.secondary)
                Divider()
                HStack {
                    Text("Approved resume").font(.headline)
                    Spacer();Button("Select PDF…") { store.chooseResume() }
                }
                Text(store.snapshot?.integrations?.resume_name.isEmpty == false ? store.snapshot!.integrations!.resume_name : "No PDF selected. Initial Gmail drafts require your approved PDF.").font(.caption).foregroundStyle(.secondary)
                Text("Initial outreach template").font(.headline)
                TextEditor(text: $initial).font(.system(size: 12)).frame(height: 130).border(.secondary.opacity(0.2))
                Text("Follow-up template").font(.headline)
                TextEditor(text: $followup).font(.system(size: 12)).frame(height: 100).border(.secondary.opacity(0.2))
                Text("Start each template with Subject: on its own line. Placeholders: {{first_name}}, {{contact_name}}, {{company}}, {{role}}, {{job_url}}, {{job_specific}}, {{your_name}}. Leave blank to configure later.")
                    .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                Button("Save templates") { save() }.buttonStyle(.borderedProminent)
                if let error = store.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            }.padding(.trailing, 6)
        }.frame(height: 500)
        .onAppear { sheet = store.snapshot?.integrations?.sheet_url ?? ""; initial = store.snapshot?.integrations?.initial_template ?? ""; followup = store.snapshot?.integrations?.followup_template ?? "" }
    }
    private func save() { Task { await store.perform(["op":"google_settings","sheet_url":sheet,"initial_template":initial,"followup_template":followup]) } }
}

struct JobEmailDetail: View {
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    private var messages: [JobMailRecord] { store.snapshot?.mail?.messages.filter { $0.job_id == job.id } ?? [] }
    private var due: String { store.snapshot?.mail?.meta.first { $0.job_id == job.id }?.follow_up ?? "" }
    @State private var dueEdit = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(job.recruiter_name).font(.headline)
            Text(job.recruiter_contact.isEmpty ? "No published email established" : job.recruiter_contact).font(.caption).textSelection(.enabled)
            HStack {
                Button("Prepare initial draft") { prepare("initial") }
                Button("Prepare follow-up") { prepare("followup") }
            }.controlSize(.small).disabled(store.busy)
            Text("Templates can be added later in Settings. Drafts preserve your wording; review them before creating them in Gmail.").font(.caption).foregroundStyle(.secondary)
            HStack {
                TextField("Follow-up date · YYYY-MM-DD", text: $dueEdit).textFieldStyle(.roundedBorder)
                Button("Save date") { Task { await store.perform(["op":"follow_up","id":job.id,"value":dueEdit]) } }.controlSize(.small)
            }
            if !due.isEmpty { Text("Follow-up due "+due).font(.caption).foregroundStyle(.secondary) }
            ForEach(messages) { message in JobEmailEditor(store: store, message: message) }
            Link("Open Gmail ↗", destination: URL(string:"https://mail.google.com/mail/u/?authuser=lalithuriti%40gmail.com#drafts")!).font(.caption)
        }.onAppear { dueEdit = due }.onChange(of: job.id) { _, _ in dueEdit = due }
    }
    private func prepare(_ kind: String) { Task { await store.perform(["op":"prepare_email","id":job.id,"kind":kind]) } }
}

private struct JobEmailEditor: View {
    @ObservedObject var store: JobAppsStore
    let message: JobMailRecord
    @State private var subject = ""
    @State private var bodyText = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack { Text(message.kind == "initial" ? "Initial outreach" : "Follow-up").font(.headline);Spacer();Text(message.state).font(.caption).foregroundStyle(.secondary) }
            Text("To: "+message.recipient).font(.caption).textSelection(.enabled)
            TextField("Subject", text: $subject).textFieldStyle(.roundedBorder).disabled(message.state != "Local draft")
            TextEditor(text: $bodyText).font(.system(size: 12)).frame(minHeight: 150).disabled(message.state != "Local draft")
            if let error = message.error, !error.isEmpty { Text(error).font(.caption).foregroundStyle(.orange) }
            if message.state == "Local draft" {
                HStack {
                    Button("Save local draft") { Task { await save() } }
                    Button("Create Gmail draft") { Task { if await save() { await store.perform(["op":"gmail_draft","id":message.id]) } } }
                        .buttonStyle(.borderedProminent).disabled(store.snapshot?.integrations?.connected != true || store.snapshot?.integrations?.busy == true)
                }.controlSize(.small).disabled(store.busy)
            } else { Text("Edit and send in Gmail. Missing or uncertain drafts are reconciled before any retry.").font(.caption).foregroundStyle(.secondary) }
        }.padding(12).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
        .onAppear { subject = message.subject; bodyText = message.body }
    }
    @discardableResult private func save() async -> Bool { await store.perform(["op":"edit_email","id":message.id,"subject":subject,"body":bodyText]) }
}
