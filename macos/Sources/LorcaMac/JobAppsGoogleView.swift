import SwiftUI

struct JobGoogleDraft {
    var sheet = ""
    var initial = ""
    var followup = ""
    var version = ""
}

struct JobGoogleSettings: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    @Binding var draft: JobGoogleDraft
    var showTemplates: Bool
    var sync: () -> Void
    @State private var followup = false
    var body: some View {
        if showTemplates {
            VStack(alignment: .leading, spacing: 12) {
                Picker("Template", selection: $followup) {
                    Text("Initial outreach").tag(false)
                    Text("Follow-up").tag(true)
                }.pickerStyle(.segmented).labelsHidden()
                JobMarkdownEditor(title: followup ? "Follow-up template" : "Outreach template",
                                  text: followup ? $draft.followup : $draft.initial)
                DisclosureGroup("Template fields") {
                    MarkdownText(text: "Start with `Subject: Your subject`, then the body. Use `{{first_name}}`, `{{company}}`, `{{role}}`, `{{job_url}}`, `{{job_specific}}`, `{{contact_name}}` or `{{your_name}}`.", size: max(10, 12 + zoom))
                        .padding(.top, 6)
                }.font(.system(size: max(10, 11 + zoom)))
            }
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    GroupBox("Google account") {
                        HStack {
                            VStack(alignment: .leading, spacing: 5) {
                                Text("lalithuriti@gmail.com").font(.system(size: max(10, 13 + zoom), weight: .medium))
                                Text(store.snapshot?.integrations?.connected == true ? "Connected · Send only after approval" : "Connect to sync jobs and prepare outreach")
                                    .font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button(store.snapshot?.integrations?.connected == true ? "Reconnect…" : "Connect…") { store.connectGoogle() }
                                .disabled(store.snapshot?.integrations?.busy == true || store.busy)
                        }.padding(8)
                    }
                    GroupBox("Google Sheet") {
                        VStack(alignment: .leading, spacing: 10) {
                            TextField("Paste your Sheet URL", text: $draft.sheet).textFieldStyle(.roundedBorder)
                            HStack {
                                Text(store.snapshot?.integrations?.message ?? "Not connected").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary).textSelection(.enabled)
                                Spacer()
                                Button(store.snapshot?.integrations?.enabled == true ? "Sync now" : "Enable sync", action: sync)
                                    .disabled(store.snapshot?.integrations?.connected != true || store.snapshot?.integrations?.busy == true || store.busy)
                            }
                            Text("Syncs on open, after saving, and every minute.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                        }.padding(8)
                    }
                    if let conflicts = store.snapshot?.integrations?.settings_conflicts, !conflicts.isEmpty {
                        GroupBox("Changes need review") {
                            VStack(alignment: .leading, spacing: 12) {
                                ForEach(conflicts) { conflict in
                                    DisclosureGroup(conflict.field.replacingOccurrences(of: "_", with: " ").capitalized) {
                                        ScrollView {
                                            MarkdownText(text: "**This Mac**\n\n" + conflict.local_value + "\n\n**Google Sheet**\n\n" + conflict.remote_value, size: max(10, 12 + zoom))
                                        }.frame(height: 150)
                                        HStack {
                                            ForEach(["local", "sheet"], id: \.self) { choice in
                                                Button(choice == "local" ? "Keep this Mac" : "Use Sheet") {
                                                    Task { await store.perform(["op": "settings_conflict", "field": conflict.field, "choice": choice]) }
                                                }
                                            }
                                        }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small).disabled(store.busy || store.running || store.snapshot?.integrations?.busy == true)
                                    }
                                }
                            }.padding(8)
                        }
                    }
                }
            }
        }
    }
}

struct JobEmailDetail: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    private var messages: [JobMailRecord] { store.snapshot?.mail?.messages.filter { $0.job_id == job.id } ?? [] }
    private var conversation: [JobMailSnapshot.ThreadMessage] { store.snapshot?.mail?.threads?[String(job.id)] ?? [] }
    private var due: String { store.snapshot?.mail?.meta.first { $0.job_id == job.id }?.follow_up ?? "" }
    @State private var dueEdit = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(store.actionLabel(job)).font(.system(size: max(10, 14 + zoom), weight: .medium)).foregroundStyle(store.actionColor(job))
                Spacer()
                Button("Refresh conversation") { refresh() }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                    .disabled(store.snapshot?.integrations?.connected != true || store.snapshot?.integrations?.busy == true)
            }
            JobContactView(store: store, job: job)
            if !conversation.isEmpty {
                ForEach(conversation) { item in
                    VStack(alignment: .leading, spacing: 7) {
                        HStack {
                            Text(item.sent ? "You" : item.from).font(.system(size: max(10, 12 + zoom), weight: .medium))
                            Spacer()
                            Text(item.sent ? "Sent" : "Received").font(.system(size: max(10, 11 + zoom))).foregroundStyle(item.sent ? Color.secondary : Color.blue)
                        }
                        Text(item.body).font(.system(size: max(10, 12 + zoom))).textSelection(.enabled)
                        Text(item.date).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                    }.padding(.vertical, 10)
                    Divider()
                }
            } else {
                ForEach(messages.filter { $0.state == "Sent" || $0.state == "Needs review" }) { message in
                    VStack(alignment: .leading, spacing: 5) {
                        Text("You → " + message.recipient).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                        Text(message.body).font(.system(size: max(10, 12 + zoom))).textSelection(.enabled)
                    }.padding(.vertical, 8)
                }
            }
            ForEach(messages.filter { $0.state != "Sent" && $0.state != "Needs review" }) { message in
                JobEmailEditor(store: store, message: message)
            }
            if !messages.contains(where: { $0.kind == "initial" }), job.recruiter_contact.contains("@") {
                Button("Prepare outreach") { prepare("initial") }.buttonStyle(.borderedProminent).disabled(store.busy)
            } else if store.actionLabel(job) == "Follow-up due", !messages.contains(where: { $0.kind == "followup" }) {
                Button("Prepare follow-up") { prepare("followup") }.buttonStyle(.borderedProminent).disabled(store.busy)
            }
            if messages.isEmpty, !job.recruiter_contact.contains("@") {
                Text("Add the recruiter's email above. Other jobs keep moving.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
            }
            if job.outreach_stage == "Replied" || messages.contains(where: { $0.state == "Needs review" }) {
                Text("Reply received · Automatic follow-up cancelled.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.blue)
                Link("Reply in Gmail ↗", destination: gmailURL).font(.system(size: max(10, 11 + zoom)))
                let receipt = store.replyReceipt(job.id)
                if !receipt.isEmpty && receipt != store.snapshot?.mail?.meta.first(where: { $0.job_id == job.id })?.reviewed_reply {
                    Button("Done reviewing reply") { Task { await store.perform(["op":"review_reply","id":job.id,"receipt":receipt]) } }
                        .disabled(store.busy)
                } else {
                    Text("Reply reviewed").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                }
            }
            DisclosureGroup(due.isEmpty ? "Follow-up schedule" : "Follow-up · " + due) {
                HStack {
                    TextField("YYYY-MM-DD", text: $dueEdit).textFieldStyle(.roundedBorder)
                    Button("Save") { Task { await store.perform(["op":"follow_up","id":job.id,"value":dueEdit]) } }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small).padding(.top, 8)
            }.font(.system(size: max(10, 11 + zoom)))
        }
        .onAppear { dueEdit = due }
        .onChange(of: job.id) { _, _ in dueEdit = due }
        .task(id: job.id) { if store.snapshot?.integrations?.connected == true && store.snapshot?.integrations?.busy != true && !messages.isEmpty { refresh() } }
    }
    private var gmailURL: URL { URL(string:"https://mail.google.com/mail/u/?authuser=lalithuriti%40gmail.com#inbox")! }
    private func refresh() { Task { await store.perform(["op":"load_thread","id":job.id]) } }
    private func prepare(_ kind: String) { Task { await store.perform(["op":"prepare_email","id":job.id,"kind":kind]) } }
}

private struct JobEmailEditor: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let message: JobMailRecord
    @State private var subject = ""
    @State private var bodyText = ""
    @State private var token = ""
    @State private var loaded = false
    private var dirty: Bool { subject != message.subject || bodyText != message.body }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(message.kind == "initial" ? "Outreach draft" : "Follow-up draft").font(.system(size: max(10, 14 + zoom), weight: .medium))
                Spacer()
                Text(message.state == "Local draft" ? "Review & send" : message.state).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
            }
            Text("To: " + message.recipient).font(.system(size: max(10, 11 + zoom))).textSelection(.enabled)
            if message.state == "Local draft" {
                TextField("Subject", text: $subject).textFieldStyle(.roundedBorder)
                TextEditor(text: $bodyText).font(.system(size: max(10, 13 + zoom))).frame(minHeight: 170)
                if message.kind == "initial" {
                    Label("Master · " + (store.snapshot?.integrations?.resume_name ?? "Choose a PDF in Candidate profile"), systemImage: "paperclip")
                        .font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                }
                HStack {
                    Button("Approve & send") { Task {
                        if await save(), let current = store.snapshot?.mail?.messages.first(where: { $0.id == message.id }), let approval = current.approval_token {
                            await store.perform(["op":"send_email","id":message.id,"approval_token":approval])
                        }
                    } }.buttonStyle(.borderedProminent)
                    Button(dirty ? "Save draft" : "Saved") { Task { await save() } }.disabled(!dirty)
                    Spacer()
                    Menu("More") {
                        Button("Create Gmail draft") { Task { if await save() { await store.perform(["op":"gmail_draft","id":message.id]) } } }
                    }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small).disabled(store.busy || store.snapshot?.integrations?.busy == true || store.snapshot?.integrations?.connected != true)
            } else if message.state == "Gmail draft" {
                Text(message.body).font(.system(size: max(10, 12 + zoom))).textSelection(.enabled)
                Link("Review & send in Gmail ↗", destination: URL(string:"https://mail.google.com/mail/u/?authuser=lalithuriti%40gmail.com#drafts")!).font(.system(size: max(10, 11 + zoom)))
            } else {
                Text("Checking Gmail before any retry. This message will not be sent again automatically.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
            }
            if let error = message.error, !error.isEmpty { Text(error).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.yellow).textSelection(.enabled) }
        }.padding(14).background(.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10))
        .onAppear {
            let draft = store.emailEdits[message.id]
            subject = draft?.subject ?? message.subject;bodyText = draft?.body ?? message.body
            token = draft?.token ?? message.approval_token ?? "";loaded = true
        }
        .onChange(of: subject) { _, _ in remember() }
        .onChange(of: bodyText) { _, _ in remember() }
        .onChange(of: message) { old, new in
            if subject == old.subject && bodyText == old.body { subject = new.subject;bodyText = new.body;token = new.approval_token ?? "" }
        }
    }
    private func remember() {
        if loaded { store.emailEdits[message.id] = .init(subject: subject, body: bodyText, token: token) }
    }
    @discardableResult private func save() async -> Bool {
        let ok = await store.perform(["op":"edit_email","id":message.id,"subject":subject,"body":bodyText,"expected_token":token])
        if ok { token = store.snapshot?.mail?.messages.first(where: { $0.id == message.id })?.approval_token ?? "";store.emailEdits.removeValue(forKey: message.id) }
        return ok
    }
}
