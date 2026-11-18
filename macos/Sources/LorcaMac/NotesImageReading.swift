import Foundation
import ImageIO
import Vision

struct NotesImageReading: Sendable {
    let images: [String]
    let observations: String
    let dates: [NotesDateCandidate]

    static func read(_ references: [NotesImageReference], directory: URL, passage: String) throws -> NotesImageReading {
        var encoded: [String] = [], observations: [String] = []
        for reference in references {
            let url = directory.appendingPathComponent(reference.id.uuidString + ".jpg")
            guard let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize, size <= AttachmentStore.maxStoredBytes else { throw ImageError.tooLarge }
            let data = try Data(contentsOf: url)
            guard let source = CGImageSourceCreateWithData(data as CFData, nil), let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw ImageError.missing }
            encoded.append(data.base64EncodedString())
            guard encoded.reduce(0, { $0 + $1.utf8.count }) <= 12_000_000 else { throw ImageError.payloadTooLarge }
            let text = VNRecognizeTextRequest()
            text.recognitionLevel = .accurate; text.usesLanguageCorrection = true
            text.preferBackgroundProcessing = true
            let handler = VNImageRequestHandler(cgImage: image)
            try handler.perform([text])
            let words = (text.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
            observations.append("Visible text:\n" + (words.isEmpty ? "No text recognized." : String(words.prefix(6000))))
        }
        let evidence = observations.joined(separator: "\n\n")
        return NotesImageReading(images: encoded, observations: evidence, dates: NotesCalendar.candidates(in: passage + "\n" + evidence))
    }
}
