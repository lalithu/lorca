import AppKit
import UniformTypeIdentifiers

// AppKit text editors otherwise consume image paste/drop events as plain text.
enum ClipboardImages {
    static func read(_ pasteboard: NSPasteboard) -> [ImageInput] {
        let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        if !urls.isEmpty {
            return urls.filter { url in
                (try? url.resourceValues(forKeys: [.contentTypeKey]).contentType?.conforms(to: .image)) == true
            }.map(ImageInput.file)
        }
        return (pasteboard.pasteboardItems ?? []).compactMap { item in
            let types: [NSPasteboard.PasteboardType] = [.png, .init(UTType.jpeg.identifier), .tiff, .init(UTType.heic.identifier)]
            for type in types {
                if let data = item.data(forType: type) { return .bytes(data, name: "Pasted image") }
            }
            return nil
        }
    }
}
