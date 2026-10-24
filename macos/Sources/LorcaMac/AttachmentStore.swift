import AppKit
import ImageIO
import UniformTypeIdentifiers

struct AttachmentStore: Sendable {
    static let maxImages = 4
    static let maxSourceBytes = 20 * 1024 * 1024
    static let maxStoredBytes = 8 * 1024 * 1024
    let directory: URL

    func importFile(_ url: URL) throws -> ChatAttachment {
        let accessed = url.startAccessingSecurityScopedResource()
        defer { if accessed { url.stopAccessingSecurityScopedResource() } }
        guard url.isFileURL else { throw ImageError.invalid }
        let size = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard size.isRegularFile == true else { throw ImageError.invalid }
        guard (size.fileSize ?? Int.max) <= Self.maxSourceBytes else { throw ImageError.tooLarge }
        return try importData(Data(contentsOf: url), name: url.lastPathComponent)
    }

    func importData(_ data: Data, name: String) throws -> ChatAttachment {
        guard data.count <= Self.maxSourceBytes else { throw ImageError.tooLarge }
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, Double(width) * Double(height) <= 40_000_000 else { throw ImageError.invalid }
        let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 4096]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary),
              let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8,
                bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { throw ImageError.invalid }
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: image.width, height: image.height))
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        guard let flattened = context.makeImage() else { throw ImageError.invalid }
        let encoded = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(encoded, UTType.jpeg.identifier as CFString, 1, nil) else { throw ImageError.invalid }
        CGImageDestinationAddImage(destination, flattened, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
        guard CGImageDestinationFinalize(destination), encoded.length <= Self.maxStoredBytes else { throw ImageError.tooLarge }
        let attachment = ChatAttachment(id: UUID(), name: String(name.prefix(200)), width: image.width, height: image.height)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try (encoded as Data).write(to: url(for: attachment), options: .atomic)
        return attachment
    }

    func url(for attachment: ChatAttachment) -> URL { directory.appendingPathComponent(attachment.fileName) }
    func data(for attachment: ChatAttachment) throws -> Data {
        let url = url(for: attachment)
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= Self.maxStoredBytes,
              let data = try? Data(contentsOf: url), CGImageSourceCreateWithData(data as CFData, nil) != nil else { throw ImageError.missing }
        return data
    }

    func thumbnail(for attachment: ChatAttachment, maxSize: Int = 300) -> NSImage? {
        guard let source = CGImageSourceCreateWithURL(url(for: attachment) as CFURL, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: maxSize] as CFDictionary) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
    }

    func remove(_ id: UUID) throws {
        let url = directory.appendingPathComponent("\(id.uuidString).jpg")
        if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
    }
}

enum ImageError: LocalizedError {
    case invalid, tooLarge, tooMany, missing, payloadTooLarge
    var errorDescription: String? {
        switch self {
        case .invalid: return "That file isn’t a readable image. Try a PNG, JPEG, HEIC, or TIFF image under 40 megapixels."
        case .tooLarge: return "Choose an image under 20 MB. Optimized images must be under 8 MB."
        case .tooMany: return "Attach up to four images per message."
        case .missing: return "A saved image is missing or unreadable. Restore it from a backup, or start a new chat."
        case .payloadTooLarge: return "This conversation has too many images for one request (24 MB maximum). Start a new chat."
        }
    }
}
