import AppKit

@MainActor
extension ChatStore {
    func attachImages(_ inputs: [ImageInput]) { importImages(inputs) }

    func attachFiles(_ urls: [URL]) { importImages(urls.map(ImageInput.file)) }

    func attachProviders(_ providers: [NSItemProvider]) {
        guard ready, !isImportingImages else { return }
        importImages([], providers: providers)
    }

    private func importImages(_ inputs: [ImageInput], providers: [NSItemProvider] = []) {
        guard ready, !isImportingImages else { return }
        guard inputs.count + providers.count + draftImages.count <= AttachmentStore.maxImages else {
            error = ImageError.tooMany.localizedDescription
            return
        }
        let chatID = library.selectedID
        let storage = imageStore
        isImportingImages = true
        Task {
            defer { isImportingImages = false }
            var pending = inputs
            do {
                for provider in providers { pending.append(try await ImageInput.load(provider)) }
            } catch { self.error = friendlyError(error); return }
            for input in pending {
                do {
                    let attachment = try await Task.detached(priority: .userInitiated) { try input.store(in: storage) }.value
                    enqueue(["op": "importImage", "conversationId": chatID.uuidString, "data": attachment.data.base64EncodedString(), "name": attachment.name])
                } catch { self.error = friendlyError(error) }
            }

        }
    }
}
