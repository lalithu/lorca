import SwiftUI
import PDFKit

struct JobApplicationView: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    let model: String
    @State private var editing = false
    @State private var changes = false
    @State private var source = ""
    @State private var selected = "master"
    private var record: JobResumeSnapshot.Record? { store.snapshot?.resumes?.records.first { $0.job_id == job.id } }
    private var work: JobResumeSnapshot.Work? { store.snapshot?.resumes?.work }
    private var preparing: Bool { work?.busy == true }
    private var tailored: Bool { selected == "tailored" }
    private var pdf: URL? {
        guard let root = store.snapshot?.root else { return nil }
        let path = tailored ? record?.pdf_path ?? "" : URL(fileURLWithPath: root).appendingPathComponent("profile/approved-resume.pdf").path
        return !path.isEmpty && FileManager.default.fileExists(atPath: path) ? URL(fileURLWithPath: path) : nil
    }
    private var version: String { tailored ? record?.version ?? "" : store.snapshot?.integrations?.resume_sha256 ?? "" }
    private var name: String { tailored ? "\(job.company)_Resume.pdf" : store.snapshot?.integrations?.resume_name ?? "Resume.pdf" }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Application resume").font(.system(size: max(10, 14 + zoom), weight: .medium))
                Spacer()
                Picker("Resume", selection: $selected) { Text("Master").tag("master");Text("Tailored").tag("tailored") }
                    .pickerStyle(.segmented).labelsHidden().frame(maxWidth: 190)
            }
            if preparing {
                HStack { ProgressView().controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small);Text(work?.message ?? "Preparing…").font(.system(size: max(10, 11 + zoom))) }
            } else if let message = work?.message, !message.isEmpty, work?.job_id == job.id {
                Text(message).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary).textSelection(.enabled)
            }
            if tailored {
                HStack {
                    Text(record == nil ? "Prepared from your master LaTeX." : name).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                    Spacer()
                    if record != nil {
                        Button(changes ? "Hide changes" : "Show changes") { changes.toggle() }
                        Button(editing ? "Close editor" : "Edit LaTeX") { if !editing { source = record?.source ?? "" };editing.toggle() }
                    }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                if changes, let record { JobResumeChanges(before: record.master_source, after: record.source) }
                if editing {
                    TextEditor(text: $source).font(.system(size: max(10, 12 + zoom), design: .monospaced)).frame(height: 220)
                    Button("Update PDF") { compile(source) }.disabled(preparing || store.busy)
                }
                if record == nil {
                    Text("Prioritizes the most relevant existing bullets for this role. Facts and wording stay yours.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                    Button("Tailor for this job") { Task { await store.perform(["op":"tailor_resume","id":job.id,"model":model]) } }
                        .buttonStyle(.borderedProminent).disabled(preparing || store.running || store.busy)
                    if store.running { Text("Pause research on this Mac to use the model for tailoring.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary) }
                } else if pdf == nil {
                    Button("Build PDF on this Mac") { compile(record?.source ?? "") }.disabled(preparing || store.busy)
                }
            }
            if let pdf {
                JobResumePDF(url: pdf, version: version).frame(height: 390)
                HStack {
                    Button("Save PDF…") { save(pdf) }
                    if !tailored { Button("Replace master…") { store.chooseResume() } }
                    Spacer()
                    if let url = URL(string: job.job_url), ["https","http"].contains(url.scheme ?? "") { Link("Open application ↗", destination: url).buttonStyle(.borderedProminent) }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            } else if !tailored {
                Button("Choose master PDF…") { store.chooseResume() }
            }
            Divider()
            HStack {
                Text("Complete the employer's form yourself.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                Spacer()
                Button(job.stage == "Applied" ? "Applied ✓" : "Mark applied") { Task { await store.perform(["op":"application_submitted","id":job.id,"version":version]) } }
                    .disabled(job.stage == "Applied" || store.busy || pdf == nil || preparing)
            }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            Text("Outreach emails always use your master resume.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
            if let record, !record.submitted_version.isEmpty { Text("Submitted version recorded with this job.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary) }
        }
        .onAppear { selected = record?.selection ?? "master" }
        .onChange(of: job.id) { _, _ in selected = record?.selection ?? "master";editing = false;changes = false }
        .onChange(of: selected) { _, value in Task { await store.perform(["op":"select_resume","id":job.id,"choice":value]) } }
        .onChange(of: record?.version) { _, _ in if record != nil { selected = record?.selection ?? "tailored" } }
    }
    private func compile(_ value: String) { Task { await store.perform(["op":"compile_resume","id":job.id,"source":value]);editing = false } }
    private func save(_ pdf: URL) {
        let panel = NSSavePanel();panel.nameFieldStringValue = name
        guard panel.runModal() == .OK, let target = panel.url else { return }
        do { try Data(contentsOf: pdf).write(to: target, options: .atomic) }
        catch { store.error = error.localizedDescription }
    }
}

private struct JobResumeChanges: View {
    @Environment(\.jobAppsZoom) private var zoom
    let before: String;let after: String
    private var lines: [String] {
        let left = before.components(separatedBy: .newlines), right = after.components(separatedBy: .newlines)
        return right.difference(from: left).map { item in
            switch item { case .remove(_, let text, _):return "− " + text;case .insert(_, let text, _):return "+ " + text }
        }
    }
    var body: some View {
        if lines.isEmpty { Text("Your master already uses this bullet order.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary) }
        else {
            ScrollView { VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in Text(line).font(.system(size: max(10, 11 + zoom), design: .monospaced)).foregroundStyle(line.hasPrefix("+") ? Color.green : Color.red).textSelection(.enabled) }
            }.frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 170)
        }
    }
}

private struct JobResumePDF: View {
    let url: URL
    let version: String
    @State private var pages: [NSImage] = []
    var body: some View {
        ScrollView {
            VStack(spacing: 10) {
                ForEach(Array(pages.enumerated()), id: \.offset) { _, page in
                    Image(nsImage: page).resizable().scaledToFit()
                }
            }.frame(maxWidth: .infinity)
        }
        .task(id: url.path + version) {
            guard let document = PDFDocument(url: url) else { pages = [];return }
            pages = (0..<min(document.pageCount, 10)).compactMap { document.page(at: $0)?.thumbnail(of: NSSize(width: 1000, height: 1400), for: .mediaBox) }
        }
    }
}
