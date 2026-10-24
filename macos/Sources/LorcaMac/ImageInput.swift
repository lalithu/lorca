import AppKit
import UniformTypeIdentifiers

enum ImageInput: Sendable {
    case file(URL)
    case bytes(Data, name: String)

    static func load(_ provider: NSItemProvider) async throws -> ImageInput {
        if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            return try await withCheckedThrowingContinuation { continuation in
                provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, error in
                    if let error { continuation.resume(throwing: error) }
                    else if let url = item as? URL { continuation.resume(returning: .file(url)) }
                    else if let data = item as? Data, let url = URL(dataRepresentation: data, relativeTo: nil) {
                        continuation.resume(returning: .file(url))
                    } else { continuation.resume(throwing: ImageError.invalid) }
                }
            }
        }
        guard let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) else { throw ImageError.invalid }
        let name = provider.suggestedName ?? "Pasted image"
        return try await withCheckedThrowingContinuation { continuation in
            provider.loadDataRepresentation(forTypeIdentifier: type) { data, error in
                if let error { continuation.resume(throwing: error) }
                else if let data { continuation.resume(returning: .bytes(data, name: name)) }
                else { continuation.resume(throwing: ImageError.invalid) }
            }
        }
    }

    func store(in storage: AttachmentStore) throws -> PreparedImage {
        switch self {
        case .file(let url): return try storage.importFile(url)
        case .bytes(let data, let name): return try storage.importData(data, name: name)
        }
    }
}
