import SwiftUI
import AppKit

// Saved work and local execution; frontier connections remain unavailable.
private struct RosterAgent: Identifiable {
    var id: String
    var name: String
    var provider: String
    var state = "Available"
    var local = false
}
struct RosterView: View {
    @ObservedObject var store: ChatStore
    var openChat: (UUID) -> Void = { _ in }
    @Environment(\.colorScheme) private var scheme
    private var tasks: [RosterWork] { store.roster.state.tasks }
    private var agents: [RosterAgent] {
        store.rosterMembers.members.map { member in
            let local = member.connection == .local
            return RosterAgent(id: member.id, name: member.name, provider: local ? store.providerName : member.provider,
                               state: local ? (tasks.contains(where: \.active) ? "Working" : store.isConnected ? "Available" : "Disconnected") : "Not connected", local: local)
        }
    }
    @StateObject private var arrangement = RosterArrangement()
    @State private var input = ""
    @State private var showRoster = false
    @State private var memberEditor: RosterMember?
    @State private var editingSelected = false
    @State private var expanded = Set<String>()
    @State private var mapVisible = true
    @State private var selected: RosterAgent?
    @State private var filter = "All"

    var body: some View {
        GeometryReader { size in
            VStack(spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 24) {
                        if size.size.width > 850 {
                            HStack(alignment: .top, spacing: 24) {
                                work.frame(maxWidth: .infinity)
                                inbox.frame(width: 260)
                            }
                        } else {
                            work
                            inbox
                        }
                        backlog
                        if let error = store.roster.state.error ?? store.roster.error {
                            Text(error).foregroundStyle(.red).textSelection(.enabled)
                        }
                        Text("Tasks are saved. Run locally one at a time; questions and approvals open in chat.")
                            .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
                    }
                    .padding(28)
                }
            }
        }
        .sheet(isPresented: $showRoster) {
            ZoomContent(zoom: store.uiZoom) {
                RosterManagement(members: store.rosterMembers, draft: memberEditor, resetLayout: { arrangement.reset() })
            }
        }
        .sheet(item: $selected) { agent in
            ZoomContent(zoom: store.uiZoom) {
            if editingSelected, let member = store.rosterMembers.members.first(where: { $0.id == agent.id }) {
                RosterMemberForm(member: member, isNew: false, save: {
                    if store.rosterMembers.save($0) { selected = nil; editingSelected = false }
                }, cancel: { editingSelected = false })
            } else {
            VStack(alignment: .leading, spacing: 16) {
                HStack { mark(agent, size: 32); Text(agent.name).font(ShellTypography.serif(25)) }
                Text(agent.state).foregroundStyle(.secondary)
                Text(agent.local ? "Uses the selected local model: \(store.selectedModel). Existing chat permissions apply." : "Frontier connections are not available in this pass.")
                    .font(.callout).foregroundStyle(.secondary)
                if agent.local {
                    ForEach(tasks.filter { $0.conversationId != nil }) { task in
                        HStack {
                            Text(task.title).lineLimit(1)
                            Spacer()
                            Text(task.label).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                HStack {
                    Button("Edit") { editingSelected = true }
                    Button("Remove", role: .destructive) { store.rosterMembers.remove(agent.id); selected = nil }
                    Spacer()
                    Button("Done") { selected = nil }.keyboardShortcut(.defaultAction)
                }
            }.padding(28).frame(width: 390)
            }
            }.onDisappear { editingSelected = false }
        }
        .tint(.lorcaBlue)
    }

    private var work: some View {
        VStack(alignment: .leading, spacing: 24) {
            VStack(alignment: .leading, spacing: 12) {
                Text("What do you need done?").font(ShellTypography.serif(19))
                TextField("Here’s what I need today…", text: $input, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .lineLimit(3...7)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                    .accessibilityLabel("Tasks for today")
                HStack {
                    Spacer()
                    Button("Save task", action: submit).buttonStyle(.borderedProminent)
                        .disabled(input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !store.ready || store.roster.busy || store.roster.state.error != nil)
                }
            }.padding(18).rosterSurface()
            teamMap
            HStack {
                Text("Today’s work").font(ShellTypography.serif(22).weight(.semibold))
                Spacer()
                RosterWorkFilter(selection: $filter).frame(width: 210)
            }
            VStack(spacing: 0) {
                ForEach(tasks.filter(matchesFilter)) { task in
                    taskRow(task)
                    if task.id != tasks.last?.id { Divider().padding(.horizontal, 16) }
                }
                if !tasks.contains(where: matchesFilter) {
                    Text("Nothing here right now.").foregroundStyle(.secondary).padding(24)
                }
            }.frame(maxWidth: .infinity, alignment: .leading).rosterSurface()
        }
    }

    private var teamMap: some View {
        VStack(spacing: 18) {
            HStack {
                Text("Roster").font(ShellTypography.serif(19))
                Spacer()
                Button { memberEditor = RosterMember(); showRoster = true } label: { Label("Add", systemImage: "plus") }
                    .buttonStyle(.borderedProminent).controlSize(.small)
                Button("Manage") { memberEditor = nil; showRoster = true }
                    .buttonStyle(.bordered).controlSize(.small)
                Button { mapVisible.toggle() } label: { Image(systemName: mapVisible ? "chevron.down" : "chevron.right") }
                    .buttonStyle(.plain).accessibilityLabel(mapVisible ? "Collapse roster" : "Expand roster")
            }
            if mapVisible {
                VStack(spacing: 0) {
                    Text("YOU").font(.system(size: 10, design: .monospaced))
                        .padding(.horizontal, 12).padding(.vertical, 5)
                        .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
                    if !agents.isEmpty {
                        RosterMap(items: agents, arrangement: arrangement, activate: { editingSelected = false; selected = $0 }) { agent in
                            node(agent)
                        }
                    } else {
                        Button("Add") { memberEditor = RosterMember(); showRoster = true }.padding(.top, 18)
                    }
                }
                Text("\(agents.count) agents · frontier connections pending")
                    .font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
            }
        }.padding(18).rosterSurface()
    }
    private func node(_ agent: RosterAgent) -> some View {
        VStack(spacing: 6) {
                mark(agent, size: 28)
                Text(agent.name).font(.system(size: 12, weight: .semibold)).lineLimit(1)
                Text(agent.local ? store.selectedModel : agent.state)
                    .font(.system(size: 9, design: .monospaced)).foregroundStyle(.secondary).lineLimit(1)
            }.frame(maxWidth: .infinity).frame(height: 78).padding(12)
                .background(agent.local ? Color.lorcaBlue.opacity(0.09) : Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(agent.local ? Color.lorcaBlue.opacity(0.35) : Color.primary.opacity(0.08)))
                .overlay(alignment: .topTrailing) { Circle().fill(agent.state == "Exhausted" ? Color.orange : agent.state == "Available" ? Color.green : Color.secondary).frame(width: 5, height: 5).padding(9) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(agent.name), \(agent.state)")
    }
    private func matchesFilter(_ task: RosterWork) -> Bool {
        filter == "All" || (filter == "Needs you" && task.needsYou) || (filter == "Done" && task.status == "completed")
    }
    private func taskRow(_ task: RosterWork) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Button {
                if expanded.contains(task.id) { expanded.remove(task.id) } else { expanded.insert(task.id) }
            } label: {
                HStack(spacing: 12) {
                    Image(systemName: task.status == "completed" ? "checkmark.circle" : task.active ? "circle.inset.filled" : "circle")
                        .foregroundStyle(task.needsYou ? Color.orange : task.status == "completed" ? Color.green : Color.lorcaBlue)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(task.title).font(ShellTypography.serif(17)).foregroundStyle(.primary).multilineTextAlignment(.leading)
                        Text(task.model ?? "Saved · ready to run locally").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    Text(task.label).font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary)
                    Image(systemName: expanded.contains(task.id) ? "chevron.down" : "chevron.right").font(.caption2).foregroundStyle(.tertiary)
                }
            }.buttonStyle(.plain)
            if expanded.contains(task.id) {
                Text(task.objective).font(.callout).textSelection(.enabled)
                if let summary = task.summary { Text(summary).font(.callout).foregroundStyle(.secondary).textSelection(.enabled) }
            }
            HStack {
                Spacer()
                if let conversation = task.conversationId {
                    if store.library.conversations.contains(where: { $0.id == conversation }) {
                    Button(task.needsYou ? "Respond in chat" : "Open chat") { openChat(conversation) }
                    } else { Text("Linked chat removed").font(.caption).foregroundStyle(.secondary) }
                } else {
                    Button("Run locally") {
                        Task {
                            let context = store.modelChoice.flatMap { store.contextPreference(for: $0) }
                            if let conversation = await store.roster.run(task, model: store.selectedModel, effort: store.thinkingEffort.rawValue, context: context) {
                                openChat(conversation)
                            }
                        }
                    }.disabled(!store.ready || !store.isConnected || store.agent.busy || store.roster.busy || store.jobs.running || store.roster.state.error != nil)
                }
            }.controlSize(.small)
        }.padding(17)
    }
    private var inbox: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Needs you").font(ShellTypography.serif(22).weight(.semibold))
                Spacer()
                Text("\(tasks.filter(\.needsYou).count)").font(.caption).foregroundStyle(.orange)
            }
            Text("Questions and approvals use the existing chat flow.").font(.caption).foregroundStyle(.secondary)
            ForEach(tasks.filter(\.needsYou)) { task in
                Divider()
                Text(task.title).font(.callout.weight(.semibold))
                if let conversation = task.conversationId {
                    Button("Open chat") { openChat(conversation) }.controlSize(.small)
                }
            }
            if !tasks.contains(where: \.needsYou) {
                Label("You’re all caught up.", systemImage: "checkmark.circle").font(.callout).foregroundStyle(.secondary)
            }
        }.padding(20).frame(maxWidth: .infinity, alignment: .leading).rosterSurface()
    }
    private var backlog: some View {
        DisclosureGroup("On Lorca’s own list") {
            VStack(alignment: .leading, spacing: 9) {
                Text("Notes & Job Apps: reconnect their pipelines to the new runtime.")
                Text("Cleanup: retire obsolete tests and terminal-chat dependencies; repair CI and docs.")
                Text("Reliability: structured recovery, complete command output, file tools and rollback.")
                Text("Sustained work: background task ownership, provider adapters and context/journal improvements.")
            }.font(.callout).foregroundStyle(.secondary).padding(.top, 12).frame(maxWidth: .infinity, alignment: .leading)
        }.font(.caption).foregroundStyle(.secondary)
    }
    @ViewBuilder private func mark(_ agent: RosterAgent, size: CGFloat) -> some View {
        if agent.local {
            Text("🫍").font(.system(size: size))
        } else if let image = logo(agent.provider) {
            Image(nsImage: image).resizable().scaledToFit().frame(width: size, height: size).accessibilityHidden(true)
        } else {
            Image(systemName: "network").font(.system(size: size * 0.75)).frame(width: size, height: size).foregroundStyle(.secondary)
        }
    }
    private func logo(_ provider: String) -> NSImage? {
        let name = provider == "Codex" ? (scheme == .dark ? "codex-dark" : "codex-light") : provider.lowercased().replacingOccurrences(of: " ", with: "-")
        let root = Bundle.main.resourceURL?.appendingPathComponent("Roster")
        if let root, let image = NSImage(contentsOf: root.appendingPathComponent(name + ".png")) { return image }
        if let url = Bundle.module.url(forResource: name, withExtension: "png", subdirectory: "Roster") { return NSImage(contentsOf: url) }
        return nil
    }
    private func submit() {
        let objective = input.trimmingCharacters(in: .whitespacesAndNewlines)
        Task { if await store.roster.add(objective) { input = "" } }
    }
}

private extension View {
    @ViewBuilder func rosterSurface() -> some View {
        #if compiler(>=6.2)
        if #available(macOS 26.0, *) {
            self.glassEffect(.regular, in: .rect(cornerRadius: 16))
        } else {
            self.background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
        }
        #else
        self.background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
        #endif
    }
}

@MainActor enum RosterPreview {
    static func open() {
        let app = NSApplication.shared
        let store = ChatStore()
        app.setActivationPolicy(.regular)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1220, height: 900), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Lorca · Roster UI preview"
        window.minSize = NSSize(width: 650, height: 550)
        window.isReleasedWhenClosed = false
        window.contentView = ZoomHostingView(rootView: RosterView(store: store).background(Color(nsColor: .windowBackgroundColor)), zoom: store.uiZoom)
        window.center(); window.makeKeyAndOrderFront(nil); app.activate(ignoringOtherApps: true)
        app.run()
        withExtendedLifetime(window) {}
    }
}
