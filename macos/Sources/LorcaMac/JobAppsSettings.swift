import SwiftUI

struct JobAppsSettings: View {
    @Environment(\.jobAppsZoom) private var zoom
    private enum Section: String, CaseIterable { case search = "Search", profile = "Profile", email = "Email", google = "Google" }
    @ObservedObject var store: JobAppsStore
    @Environment(\.dismiss) private var dismiss
    @State private var section: Section = .search
    @State private var preferences = ""
    @State private var profile = ""
    @State private var version = ""
    @State private var google = JobGoogleDraft()
    @State private var loaded = false
    @State private var savedProfile = ""
    @State private var savedPreferences = ""
    @State private var savedGoogle = JobGoogleDraft()
    @State private var confirmingDiscard = false
    private var profileChanged: Bool { profile != savedProfile || preferences != savedPreferences }
    private var googleChanged: Bool { google.sheet != savedGoogle.sheet || google.initial != savedGoogle.initial || google.followup != savedGoogle.followup }
    private var hasChanges: Bool { profileChanged || googleChanged }
    private var profileSection: Bool { section == .search || section == .profile }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Job Apps settings").font(.system(size: 20 + zoom, weight: .medium))
            Picker("Section", selection: $section) {
                ForEach(Section.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }.pickerStyle(.segmented).labelsHidden()
            Group {
                if !loaded { ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity) }
                else if profileSection {
                    VStack(spacing: 14) {
                        if section == .profile {
                            JobResumeDropZone(store: store)
                            JobMasterSourceView(store: store)
                        }
                        JobMarkdownEditor(title: section == .search ? "Search preferences" : "Candidate profile",
                                          text: section == .search ? $preferences : $profile)
                            .id(section)
                    }
                } else {
                    JobGoogleSettings(store: store, draft: $google, showTemplates: section == .email) {
                        Task { await saveGoogle(enableSync: true) }
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            JobAppsErrorView(store: store)
            HStack {
                Text(profileChanged && store.running ? "Pause research before saving." : hasChanges ? "Unsaved changes" : "Saved · Syncs automatically")
                    .font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                Spacer()
                Button("Done") { if hasChanges { confirmingDiscard = true } else { dismiss() } }.keyboardShortcut(.cancelAction)
                Button("Save") { Task { await save() } }.buttonStyle(.borderedProminent)
                    .disabled(!loaded || !hasChanges || store.busy || store.snapshot?.integrations?.busy == true || profileChanged && store.running)
            }
        }.padding(24).frame(width: 620 + max(0, zoom) * 12, height: 580 + max(0, zoom) * 12)
        .interactiveDismissDisabled(hasChanges)
        .confirmationDialog("Discard unsaved changes?", isPresented: $confirmingDiscard) {
            Button("Discard changes", role: .destructive) { dismiss() }
            Button("Cancel", role: .cancel) { }
        }
        .task {
            if store.snapshot == nil { await store.perform(["op": "snapshot"]) }
            guard let snapshot = store.snapshot else { return }
            preferences = snapshot.preferences.text; profile = snapshot.profile; version = snapshot.preferences.version
            google = JobGoogleDraft(sheet: snapshot.integrations?.sheet_url ?? "", initial: snapshot.integrations?.initial_template ?? "",
                                    followup: snapshot.integrations?.followup_template ?? "", version: snapshot.integrations?.templates_version ?? "")
            savedProfile = profile; savedPreferences = preferences; savedGoogle = google
            loaded = true
        }
    }
    private func save() async {
        // Keep drafts when switching tabs; Save applies all edited sections.
        guard !profileChanged || !store.running else { return }
        if googleChanged, !(await saveGoogle()) { return }
        if profileChanged, await store.perform(["op": "settings", "preferences": preferences, "profile": profile, "version": version]) {
            version = store.snapshot?.preferences.version ?? version
            savedProfile = profile; savedPreferences = preferences
        }
    }
    @discardableResult private func saveGoogle(enableSync: Bool = false) async -> Bool {
        guard await store.perform(["op": "google_settings", "sheet_url": google.sheet, "initial_template": google.initial,
                                   "followup_template": google.followup, "templatesVersion": google.version]) else { return false }
        google.version = store.snapshot?.integrations?.templates_version ?? google.version
        savedGoogle = google
        if enableSync && store.snapshot?.integrations?.enabled != true { return await store.perform(["op": "google_enable"]) }
        return true
    }
}

struct JobMasterSourceView: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    @State private var source = ""
    @State private var editing = false
    var body: some View {
        DisclosureGroup("Master LaTeX source", isExpanded: $editing) {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Button("Choose .tex…") { store.chooseLatex() }
                    Spacer()
                    Button("Save source") { Task { await store.perform(["op":"resume_source","source":source]) } }
                        .disabled(source == store.snapshot?.integrations?.latex_source || store.busy)
                    Button("Build master PDF") { Task {
                        if await store.perform(["op":"resume_source","source":source]) { await store.perform(["op":"compile_master"]) }
                    } }.disabled(source.isEmpty || store.busy || store.snapshot?.resumes?.work.busy == true)
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
                TextEditor(text: $source).font(.system(size: max(10, 11 + zoom), design: .monospaced)).frame(height: 130)
                Text("Source syncs between Macs. Build master PDF after editing, or replace the PDF above.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                if let work = store.snapshot?.resumes?.work, work.job_id == 0 { Text(work.message).font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary) }
            }.padding(.top, 8)
        }.font(.system(size: max(10, 11 + zoom)))
        .onAppear { source = store.snapshot?.integrations?.latex_source ?? "" }
        .onChange(of: store.snapshot?.integrations?.latex_source) { old, new in
            if source == old || source.isEmpty { source = new ?? "" }
        }
    }
}

struct JobMarkdownEditor: View {
    @Environment(\.jobAppsZoom) private var zoom
    let title: String
    @Binding var text: String
    @State private var editing = false
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(title).font(.system(size: max(10, 14 + zoom), weight: .medium))
                Spacer()
                Picker("Display", selection: $editing) { Text("Preview").tag(false); Text("Edit").tag(true) }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 140)
            }
            Group {
                if editing {
                    TextEditor(text: $text).font(.system(size: max(10, 13 + zoom))).padding(8)
                } else {
                    ScrollView {
                        if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text("Choose Edit to add your text.").font(.system(size: max(10, 13 + zoom))).foregroundStyle(.secondary)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(12)
                        } else { MarkdownText(text: text, size: max(10, 13 + zoom)).padding(12) }
                    }
                }
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
                .overlay { RoundedRectangle(cornerRadius: 8).stroke(.secondary.opacity(0.16), lineWidth: 1) }
        }
    }
}

struct JobResumeDropZone: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    @State private var targeted = false
    private var name: String { store.snapshot?.integrations?.resume_name ?? "" }
    var body: some View {
        VStack(spacing: 7) {
            Label(name.isEmpty ? "Drop your resume PDF here" : name, systemImage: "doc.richtext")
                .font(.system(size: max(10, 13 + zoom), weight: .medium)).lineLimit(1).truncationMode(.middle)
            HStack(spacing: 6) {
                Text(name.isEmpty ? "or" : "Replace it:").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.secondary)
                Button("Choose PDF…") { store.chooseResume() }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            }
            Text("For initial email drafts · PDF up to 15 MB · Stored on this Mac")
                .font(.system(size: max(10, 10 + zoom))).foregroundStyle(.secondary)
            if store.snapshot?.integrations?.resume_conflict == true {
                Text("Both Macs changed the master PDF. Choose which one to keep.").font(.system(size: max(10, 11 + zoom))).foregroundStyle(.yellow)
                HStack {
                    ForEach(["local", "sheet"], id: \.self) { choice in
                        Button(choice == "local" ? "Use this Mac" : "Use Sheet") { Task { await store.perform(["op":"resume_sync_choice","choice":choice]) } }
                    }
                }.controlSize(zoom >= 5 ? .large : zoom >= 2 ? .regular : .small)
            }
        }.frame(maxWidth: .infinity).padding(14)
            .background(targeted ? Color.accentColor.opacity(0.09) : Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
            .overlay { RoundedRectangle(cornerRadius: 8).stroke(targeted ? Color.accentColor : .secondary.opacity(0.3), style: StrokeStyle(lineWidth: 1, dash: [5, 3])) }
            .dropDestination(for: URL.self) { urls, _ in store.importResume(urls) } isTargeted: { targeted = $0 }
            .disabled(store.busy || store.snapshot?.integrations?.busy == true)
            .accessibilityLabel("Resume PDF drop area")
    }
}
