import AppKit
import SwiftUI

struct AttachmentStrip: View {
    let attachments: [ChatAttachment]
    let storage: AttachmentStore
    var compact = false
    var remove: ((UUID) -> Void)?

    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(attachments) { attachment in
                    AttachmentPreview(attachment: attachment, storage: storage, height: compact ? 52 : 88)
                        .overlay(alignment: .topTrailing) {
                            if let remove {
                                Button { remove(attachment.id) } label: {
                                    Image(systemName: "xmark.circle.fill").symbolRenderingMode(.palette)
                                        .foregroundStyle(.white, .black.opacity(0.7))
                                }
                                .buttonStyle(.plain).padding(3)
                                .accessibilityLabel("Remove image: \(attachment.name)")
                            }
                        }
                }
            }
        }
        .scrollIndicators(.hidden).frame(height: compact ? 52 : 88)
    }
}

private struct AttachmentPreview: View {
    let attachment: ChatAttachment
    let storage: AttachmentStore
    let height: CGFloat
    @State private var image: NSImage?
    var body: some View {
        Button {
            if image != nil { NSWorkspace.shared.open(storage.url(for: attachment)) }
        } label: {
            Group {
                if let image { Image(nsImage: image).resizable().scaledToFit() }
                else {
                    VStack(spacing: 4) {
                        Image(systemName: "photo.badge.exclamationmark")
                        Text("Image unavailable").font(.system(size: 9))
                    }
                    .frame(width: height * 1.3, height: height).foregroundStyle(.secondary)
                }
            }
            .frame(height: height).frame(maxWidth: height * 2)
            .background(.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.primary.opacity(0.1)))
        }
        .buttonStyle(.plain).help(attachment.name)
        .accessibilityLabel("Image: \(attachment.name)")
        .task(id: attachment.id) { image = storage.thumbnail(for: attachment) }
    }
}
