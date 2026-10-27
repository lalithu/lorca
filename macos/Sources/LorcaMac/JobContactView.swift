import SwiftUI

struct JobContactView: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    @State private var editing = false
    @State private var name = ""
    @State private var email = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(job.recruiter_name.isEmpty ? "Recruiter" : job.recruiter_name).font(.system(size: max(10, 14 + zoom), weight: .medium))
                Spacer()
                Button(job.recruiter_name.isEmpty ? "Add recruiter" : "Edit recruiter") { beginEditing() }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            }
            if editing {
                TextField("Recruiter name", text: $name).textFieldStyle(.roundedBorder)
                TextField("Email · optional", text: $email).textFieldStyle(.roundedBorder)
                Text("Provided by you · Used for future outreach. Existing Gmail messages keep their recipient.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                HStack {
                    Button("Save recruiter") {
                        Task { if await store.perform(["op":"save_contact", "id":job.id,"name":name,"email":email]) { editing = false } }
                    }.buttonStyle(.borderedProminent).disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || store.busy)
                    Button("Cancel") { editing = false }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            } else if job.recruiter_contact.contains("@") {
                Text(job.recruiter_contact).font(.system(size: max(10, 11 + zoom))).textSelection(.enabled)
            } else {
                Button("Add recruiter email") { beginEditing() }.foregroundStyle(.yellow).controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                if let url = URL(string: job.recruiter_contact), url.scheme == "https" { Link("View public profile ↗", destination: url).font(.system(size: max(10, 11 + zoom))) }
            }
        }.onChange(of: job.id) { _, _ in editing = false }
    }
    private func beginEditing() {
        name = job.recruiter_name
        email = job.recruiter_contact.contains("@") ? job.recruiter_contact : ""
        editing = true
    }
}
