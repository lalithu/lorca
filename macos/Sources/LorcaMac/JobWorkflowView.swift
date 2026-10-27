import SwiftUI

struct JobWorkflowView: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    private let steps = ["Apply", "Recruiter", "Review in Gmail", "Waiting"]
    var body: some View {
        let current = store.workflowStep(job)
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                ForEach(steps.indices, id: \.self) { index in
                    Label(steps[index], systemImage: index < current ? "checkmark.circle.fill" : index == current ? "circle.inset.filled" : "circle")
                        .foregroundStyle(index < current || current == 3 ? Color.green : index == current ? Color.accentColor : Color.secondary)
                        .font(.system(size: max(10, 11 + zoom), weight: index == current ? .semibold : .regular))
                }
            }.fixedSize(horizontal: false, vertical: true)
            Text(current == 0 ? "Finish the application, then mark it Applied."
                 : current == 1 ? "Add the recruiter you found. Your saved contact syncs to both Macs."
                 : current == 2 ? "Prepare the draft, review it in Gmail, and send when you're ready. Lorca checks Gmail automatically."
                 : store.waitingSummary(job))
                .font(.system(size: max(10, 12 + zoom))).foregroundStyle(current == 3 ? Color.green : Color.secondary)
        }.frame(maxWidth: .infinity, alignment: .leading).padding(14)
            .background(Color.accentColor.opacity(0.045), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct JobNotesView: View {
    @Environment(\.jobAppsZoom) private var zoom
    @ObservedObject var store: JobAppsStore
    let job: JobRecord
    @State private var notes = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Notes").font(.system(size: max(10, 12 + zoom), weight: .semibold))
            MarkdownNoteEditor(text: $notes, size: max(10, 13 + zoom)).frame(height: 230)
                .background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 8))
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay { RoundedRectangle(cornerRadius: 8).stroke(.secondary.opacity(0.16), lineWidth: 1) }
            HStack {
                Menu(job.priority + " priority") {
                    ForEach(store.snapshot?.priorities ?? [], id: \.self) { value in
                        Button(value) { store.update(job, field: "priority", value: value) }
                    }
                }
                Spacer()
                Button(notes == job.notes ? "Saved" : "Save notes") {
                    Task { await store.perform(["op":"update", "id":job.id, "field":"notes", "value":notes]) }
                }.disabled(notes == job.notes || store.busy)
            }.controlSize(.small)
        }.font(.system(size: max(10, 12 + zoom)))
            .onAppear { notes = job.notes }
            .onChange(of: job.notes) { old, new in if notes == old { notes = new } }
            .onChange(of: job.id) { _, _ in notes = job.notes }
    }
}
