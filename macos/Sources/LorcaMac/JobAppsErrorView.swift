import SwiftUI

struct JobAppsErrorView: View {
    @ObservedObject var store: JobAppsStore
    var body: some View {
        if let error = store.error {
            HStack(alignment: .top, spacing: 10) {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button { store.dismissError() } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain).foregroundStyle(.secondary)
                    .help("Dismiss error").accessibilityLabel("Dismiss Job Apps error")
            }
        }
    }
}
