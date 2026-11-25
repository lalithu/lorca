import SwiftUI
import Combine

struct RosterMember: Codable, Identifiable, Equatable {
    enum Connection: String, Codable, CaseIterable {
        case none, local, endpoint, command
        var label: String {
            switch self {
            case .none: return "Not configured"
            case .local: return "Current local model"
            case .endpoint: return "HTTP endpoint"
            case .command: return "Command"
            }
        }
    }
    var id = UUID().uuidString
    var name = ""
    var provider = "Codex"
    var connection: Connection = .none
    var address = ""
    var model = ""
    static let providers = ["Codex", "Claude", "Gemini", "GitHub Copilot", "Antigravity", "Ollama", "llama.cpp", "OpenAI-compatible", "Custom"]
    var validation: String? {
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "Enter a name." }
        if connection == .command && address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "Enter a command." }
        if connection == .endpoint {
            guard let url = URL(string: address.trimmingCharacters(in: .whitespacesAndNewlines)),
                  ["http", "https"].contains(url.scheme), url.host != nil else { return "Enter an HTTP or HTTPS endpoint." }
            if url.user != nil || url.password != nil { return "Keep credentials out of the endpoint URL." }
        }
        return nil
    }
}

// Connection profiles only: saving a member never connects or executes a command.
@MainActor final class RosterMembers: ObservableObject {
    static let preferenceKey = "roster.members"
    @Published private(set) var members: [RosterMember]
    @Published private(set) var error: String?
    private let defaults: UserDefaults
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        if let data = defaults.data(forKey: Self.preferenceKey) {
            do { members = try JSONDecoder().decode([RosterMember].self, from: data) }
            catch { members = []; self.error = "Saved roster could not be read. Its original settings were preserved." }
        } else {
            members = [RosterMember(id: "lorca-local", name: "Lorca", provider: "Ollama", connection: .local),
                       RosterMember(id: "Codex", name: "Codex"),
                       RosterMember(id: "Claude", name: "Claude", provider: "Claude"),
                       RosterMember(id: "Gemini", name: "Gemini", provider: "Gemini")]
        }
    }
    @discardableResult func save(_ member: RosterMember) -> Bool {
        guard error == nil, member.validation == nil else { return false }
        var member = member
        member.name = member.name.trimmingCharacters(in: .whitespacesAndNewlines)
        member.address = member.address.trimmingCharacters(in: .whitespacesAndNewlines)
        var updated = members
        if let index = updated.firstIndex(where: { $0.id == member.id }) { updated[index] = member }
        else { updated.append(member) }
        return persist(updated)
    }
    @discardableResult func remove(_ id: String) -> Bool { persist(members.filter { $0.id != id }) }
    private func persist(_ updated: [RosterMember]) -> Bool {
        guard error == nil, let data = try? JSONEncoder().encode(updated) else { return false }
        defaults.set(data, forKey: Self.preferenceKey)
        members = updated
        return true
    }
}

struct RosterMemberForm: View {
    @State var member: RosterMember
    var isNew: Bool
    var save: (RosterMember) -> Void
    var cancel: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(isNew ? "Add to Roster" : "Edit member").font(ShellTypography.serif(25))
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 14) {
                GridRow {
                    Text("Name")
                    TextField("Name", text: $member.name).labelsHidden()
                }
                GridRow {
                    Text("Provider")
                    Picker("Provider", selection: $member.provider) {
                        ForEach(RosterMember.providers, id: \.self) { Text($0) }
                    }.labelsHidden()
                }
                GridRow {
                    Text("Connection")
                    Picker("Connection", selection: $member.connection) {
                        ForEach(RosterMember.Connection.allCases, id: \.self) { Text($0.label) }
                    }.labelsHidden()
                }
                if member.connection == .endpoint || member.connection == .command {
                    GridRow {
                        Text(member.connection == .endpoint ? "Endpoint" : "Command")
                        TextField("Endpoint or command", text: $member.address).labelsHidden()
                    }
                    GridRow {
                        Text("Model")
                        TextField("Optional", text: $member.model).labelsHidden()
                    }
                }
            }.textFieldStyle(.roundedBorder)
            Text(member.connection == .local ? "Uses the model selected in Chats. Removing this card does not affect chat." : "Settings are saved only. Connections and delegation will be wired next.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Cancel", action: cancel).keyboardShortcut(.cancelAction)
                Spacer()
                Button(isNew ? "Add" : "Save") { save(member) }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(member.validation != nil)
                    .help(member.validation ?? "Save member")
            }
        }.padding(24).frame(width: 440)
    }
}

struct RosterManagement: View {
    @ObservedObject var members: RosterMembers
    @State var draft: RosterMember?
    var resetLayout: () -> Void
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        if let draft {
            RosterMemberForm(member: draft, isNew: !members.members.contains { $0.id == draft.id }, save: {
                if members.save($0) { self.draft = nil }
            }, cancel: { self.draft = nil }).id(draft.id)
        } else {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Roster").font(ShellTypography.serif(25))
                    Spacer()
                    Button { draft = RosterMember() } label: { Label("Add", systemImage: "plus") }
                        .buttonStyle(.borderedProminent).disabled(members.error != nil)
                }
                ScrollView {
                    VStack(spacing: 12) {
                        ForEach(members.members) { member in
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(member.name).font(.body.weight(.semibold))
                                    Text("\(member.provider) · \(member.connection.label)").font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                                Button("Edit") { draft = member }
                                Button("Remove", role: .destructive) { members.remove(member.id) }
                            }
                            Divider()
                        }
                        if members.members.isEmpty { Text("Add someone to your roster.").foregroundStyle(.secondary).padding() }
                    }
                }.frame(maxHeight: 280)
                if let error = members.error { Text(error).font(.caption).foregroundStyle(.red) }
                HStack {
                    Button("Reset layout", action: resetLayout)
                    Spacer()
                    Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
                }
            }.padding(24).frame(width: 480)
        }
    }
}
