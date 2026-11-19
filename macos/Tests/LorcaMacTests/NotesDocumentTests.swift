import AppKit
import XCTest
@testable import LorcaMac

@MainActor
final class NotesDocumentTests: XCTestCase {
    private func root() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent("lorca-notes-document-\(UUID())") }
    private func flushUndo() { RunLoop.main.run(until: Date().addingTimeInterval(0.015)) }

    func testFullModelContentIsEditableUndoableAndRestoredWithoutResponseCards() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root)
        let source = "Study guide on GNN architecture"
        store.updateText(source, selection: NSRange(location: (source as NSString).length, length: 0))
        let document = store.document(for: store.selectedID)
        let proposal = NotesProposal(kind: "breakdown", source: source, title: "GNN guide",
            text: "### Message passing\nNodes aggregate messages from neighbors.\n\n**Aggregation** combines incoming messages.", headers: nil, rows: nil)
        try document.add(proposal)
        flushUndo()
        XCTAssertTrue(document.editor.string.contains("Aggregation"))
        XCTAssertTrue(document.editor.isEditable)
        XCTAssertTrue(document.editor.subviews.compactMap { $0 as? NotesInlineActivity }.isEmpty)
        XCTAssertEqual(store.current.passages.map(\.source), [source], "Model output must not recursively trigger itself")
        XCTAssertEqual(store.current.modelBlocks?.count, 1)
        let range = (document.editor.string as NSString).range(of: "incoming messages")
        document.editor.setSelectedRange(range)
        document.editor.insertText("nearby nodes", replacementRange: range)
        XCTAssertTrue(store.current.text.contains("nearby nodes"))
        XCTAssertTrue(store.current.modelBlocks?.first?.edited == true)
        let bodyFont = document.editor.textStorage?.attribute(.font, at: range.location, effectiveRange: nil) as? NSFont
        XCTAssertEqual(bodyFont?.pointSize, 16)
        let authorFont = document.editor.textStorage?.attribute(.font, at: 0, effectiveRange: nil) as? NSFont
        XCTAssertEqual(authorFont?.fontName, ShellTypography.nativeSerif(18).fontName)
        let restored = NotesStore(root: root)
        XCTAssertEqual(restored.current.modelBlocks, store.current.modelBlocks)
        XCTAssertEqual(restored.document(for: restored.selectedID).editor.string, store.current.text)
    }

    func testContributionUndoSurvivesSwitchingNotesAndDoesNotTakeFocus() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root), first = store.selectedID
        let source = "Study guide for NLP"
        store.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = store.document(for: first)
        store.create()
        let second = store.selectedID
        try document.add(NotesProposal(kind: "breakdown", source: source, title: "NLP", text: "Attention weights relevant tokens.", headers: nil, rows: nil))
        flushUndo()
        XCTAssertEqual(store.selectedID, second)
        XCTAssertEqual(store.current.text, "")
        store.select(first)
        XCTAssertTrue(store.document(for: first) === document)
        XCTAssertTrue(document.editor.undoManager!.canUndo)
        document.editor.undoManager!.undo()
        XCTAssertEqual(store.current.text, source)
        document.editor.undoManager!.redo()
        XCTAssertTrue(store.current.text.contains("Attention weights"))
    }

    func testAutomaticTableIsTheNativeEditableTableAndKeepsEveryValue() throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root)
        let source = "Table with 10,1,2,3,3,4,4,5,5,5,5"
        store.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = store.document(for: store.selectedID)
        let rows = ["10","1","2","3","3","4","4","5","5","5","5"].map { [$0] }
        try document.add(NotesProposal(kind: "table", source: source, title: "List of values", text: nil, headers: ["Value"], rows: rows))
        XCTAssertTrue(document.editor.subviews.compactMap { $0 as? NSButton }.isEmpty)
        let value = (document.editor.string as NSString).range(of: "Value")
        let style = document.editor.textStorage?.attribute(.paragraphStyle, at: value.location, effectiveRange: nil) as? NSParagraphStyle
        XCTAssertTrue(style?.textBlocks.first is NSTextTableBlock)
        let stored = try XCTUnwrap(store.current.richText)
        let decoded = try NSAttributedString(data: stored, options: [.documentType:NSAttributedString.DocumentType.rtf], documentAttributes: nil)
        XCTAssertEqual(decoded.string, document.editor.string)
        XCTAssertEqual(store.current.passages.map(\.source), [source])
    }

    static func flyerJPEG() throws -> Data {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1100, pixelsHigh: 900, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 32))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSColor.white.setFill(); NSRect(x: 0, y: 0, width: 1100, height: 900).fill()
        let lines = ["NLP STUDY NIGHT", "November 21, 2026 at 7:00 PM", "NoDa Community Room", "Free entry - bring your laptop"]
        for (i, line) in lines.enumerated() {
            (line as NSString).draw(at: NSPoint(x: 60, y: 730 - i * 150), withAttributes: [.font:NSFont.systemFont(ofSize: i == 0 ? 56 : 42), .foregroundColor:NSColor.black])
        }
        NSGraphicsContext.restoreGraphicsState()
        return try XCTUnwrap(bitmap.representation(using: .jpeg, properties: [.compressionFactor:0.9]))
    }

    static func photoJPEG() throws -> Data {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 800, pixelsHigh: 600, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 32))
        NSGraphicsContext.saveGraphicsState(); defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = try XCTUnwrap(NSGraphicsContext(bitmapImageRep: bitmap))
        NSColor(calibratedRed: 0.08, green: 0.28, blue: 0.8, alpha: 1).setFill()
        NSRect(x: 0, y: 0, width: 800, height: 600).fill()
        NSColor.red.setFill()
        let triangle = NSBezierPath(); triangle.move(to: NSPoint(x: 400, y: 520)); triangle.line(to: NSPoint(x: 100, y: 100)); triangle.line(to: NSPoint(x: 700, y: 100)); triangle.close(); triangle.fill()
        return try XCTUnwrap(bitmap.representation(using: .jpeg, properties: [.compressionFactor:0.9]))
    }

    func testImagePasteRoundTripsWithoutEmbeddingImageBytesInWorkspaceJSON() async throws {
        _ = NSApplication.shared
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root), document = store.document(for: store.selectedID)
        let data = try Self.flyerJPEG()
        if let path = ProcessInfo.processInfo.environment["LORCA_FLYER_FIXTURE"] { try data.write(to: URL(fileURLWithPath: path)) }
        document.paste([.bytes(data, name: "Flyer")])
        for _ in 0..<100 {
            if !(store.current.images ?? []).isEmpty { break }
            try await Task.sleep(for: .milliseconds(20))
        }
        let reference = try XCTUnwrap(store.current.images?.first, "Paste error: \(store.error ?? "none"); text: \(document.editor.string)")
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.mediaDirectory.appendingPathComponent(reference.id.uuidString + ".jpg").path))
        XCTAssertLessThan(try Data(contentsOf: store.file).count, data.count)
        let reopened = NotesStore(root: root)
        let editor = reopened.document(for: reopened.selectedID).editor
        XCTAssertNotNil(editor.textStorage?.attribute(.attachment, at: reference.location, effectiveRange: nil))
        XCTAssertEqual(reopened.current.text, store.current.text)
        let directory = store.mediaDirectory
        let reading = try await Task.detached { try NotesImageReading.read([reference], directory: directory, passage: "Pasted flyer") }.value
        XCTAssertTrue(reading.observations.localizedCaseInsensitiveContains("NLP STUDY NIGHT"))
        XCTAssertFalse(reading.dates.isEmpty, reading.observations)
        if let path = ProcessInfo.processInfo.environment["LORCA_FLYER_FIXTURE"] { try data.write(to: URL(fileURLWithPath: path)) }
    }

    func testLiveFlyerUsingInstalledModel() async throws {
        guard let model = ProcessInfo.processInfo.environment["LORCA_LIVE_NOTES_MODEL"] else { throw XCTSkip("Opt-in test uses an installed local model and a disposable backend") }
        _ = NSApplication.shared
        let root = root()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let process = Process()
        let checkout = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        process.executableURL = checkout.appendingPathComponent("target/debug/lorca")
        process.arguments = ["serve"]
        process.environment = ProcessInfo.processInfo.environment.merging(["LORCA_DATA_DIR":root.path]) { _, new in new }
        process.standardOutput = FileHandle.nullDevice; process.standardError = FileHandle.nullDevice
        try process.run()
        defer {
            if process.isRunning { process.terminate() }
            if ProcessInfo.processInfo.environment["LORCA_NOTES_KEEP_ROOT"] == nil { try? FileManager.default.removeItem(at: root) }
        }
        for _ in 0..<100 {
            if FileManager.default.fileExists(atPath: root.appendingPathComponent("runtime/endpoint.json").path) { break }
            try await Task.sleep(for: .milliseconds(50))
        }
        let suite = "lorca.live-notes.\(UUID())"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.set(model, forKey: "model")
        defer { defaults.removePersistentDomain(forName: suite) }
        let backend = BackendClient(root: root), chat = ChatStore(backend: backend, defaults: defaults)
        defer { chat.stop() }
        await chat.refreshConnection()
        XCTAssertTrue(chat.ready)
        let notes = NotesStore(root: root), following = NotesFollowing(notes: notes, chat: chat, delayNanoseconds: 5_000_000)
        let document = notes.document(for: notes.selectedID)
        following.setVisible(true)
        let photo = ProcessInfo.processInfo.environment["LORCA_LIVE_NOTES_PHOTO"] != nil
        document.paste([.bytes(try photo ? Self.photoJPEG() : Self.flyerJPEG(), name: photo ? "Photo" : "Study night flyer")])
        for _ in 0..<1200 {
            if photo ? !(notes.current.modelBlocks ?? []).isEmpty : notes.current.text.contains("Add to Calendar") { break }
            if following.inline.contains(where: { $0.detail != nil }) { XCTFail(following.inline.compactMap(\.detail).joined(separator: "; ")); break }
            try await Task.sleep(for: .milliseconds(100))
        }
        if photo {
            XCTAssertTrue(notes.current.text.localizedCaseInsensitiveContains("triangle"), notes.current.text)
            XCTAssertTrue(notes.current.text.localizedCaseInsensitiveContains("red"), notes.current.text)
            XCTAssertFalse(notes.current.text.contains("Add to Calendar"))
        } else { XCTAssertTrue(notes.current.text.contains("Add to Calendar"), "\(following.status)\n\(notes.current.text)") }
        XCTAssertEqual(notes.current.modelBlocks?.count, 1)
        XCTAssertEqual(notes.current.images?.count, 1)
        if !photo { XCTAssertTrue(notes.current.text.localizedCaseInsensitiveContains("free")) }
        XCTAssertTrue(document.editor.isEditable)
        XCTAssertTrue(document.editor.subviews.compactMap { $0 as? NotesInlineActivity }.allSatisfy { $0.isHidden })
        if let output = ProcessInfo.processInfo.environment["LORCA_LIVE_NOTES_OUTPUT"] {
            try JSONEncoder().encode(notes.current).write(to: URL(fileURLWithPath: output))
            document.editor.appearance = NSAppearance(named: .darkAqua)
            document.editor.frame.size = NSSize(width: 760, height: 1000)
            document.editor.layout()
            let canvas = NSBox(frame: document.editor.frame)
            canvas.boxType = .custom; canvas.contentViewMargins = .zero
            canvas.fillColor = .textBackgroundColor; canvas.appearance = NSAppearance(named: .darkAqua)
            canvas.contentView = document.editor
            if let bitmap = canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds) {
                canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
                try bitmap.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: output + ".png"))
            }
        }
        if ProcessInfo.processInfo.environment["LORCA_NOTES_KEEP_ROOT"] != nil { print("ISOLATED_NOTES_ROOT=" + root.path) }
        following.setVisible(false)
        notes.previewsPaused = true
        _ = try? await backend.command(["op":"close", "unload":false])
    }

    func testCalendarDatesRequireEvidenceAndICSIsEscapedAndOneClick() throws {
        _ = NSApplication.shared
        XCTAssertTrue(NotesCalendar.candidates(in: "November 21 at 7pm").isEmpty)
        XCTAssertTrue(NotesCalendar.candidates(in: "November 21, 2026").isEmpty)
        let dates = NotesCalendar.candidates(in: "November 21, 2026 at 7:00 PM")
        let date = try XCTUnwrap(dates.first)
        let root = root(); defer { try? FileManager.default.removeItem(at: root) }
        let store = NotesStore(root: root), source = "Pasted flyer for an NLP study night"
        store.updateText(source, selection: NSRange(location: 0, length: 0))
        let document = store.document(for: store.selectedID)
        let event = NotesCalendarEvent(title: "Study; night, 🐬\nDetails", start: date.start, end: date.end, timeZone: date.timeZone, location: "NoDa, Charlotte")
        let proposal = NotesProposal(kind: "caption", source: source, title: "NLP study night", text: "NLP study night. Free entry.\nNovember 21, 2026 at 7:00 PM.", headers: nil, rows: nil, event: event)
        try document.add(proposal)
        let link = (document.editor.string as NSString).range(of: "Add to Calendar")
        XCTAssertNotEqual(link.location, NSNotFound)
        let url = try XCTUnwrap(document.editor.textStorage?.attribute(.link, at: link.location, effectiveRange: nil) as? URL)
        let ics = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(ics.contains("SUMMARY:Study\\; night\\, 🐬\\nDetails\r\n"))
        XCTAssertTrue(ics.contains("DTSTART:"))
        XCTAssertFalse(ics.contains("DTEND:"), "No end time was provided; no duration is invented")
        XCTAssertTrue(ics.components(separatedBy: "\r\n").allSatisfy { $0.utf8.count <= 75 })
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 600), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        window.contentView = document.editor
        var clicked: URL?
        document.editor.onCalendarOpen = { clicked = $0 }
        let manager = try XCTUnwrap(document.editor.layoutManager), container = try XCTUnwrap(document.editor.textContainer)
        manager.ensureLayout(for: container)
        let glyphs = manager.glyphRange(forCharacterRange: NSRange(location: link.location, length: 1), actualCharacterRange: nil)
        let rect = manager.boundingRect(forGlyphRange: glyphs, in: container)
        let location = document.editor.convert(NSPoint(x: rect.midX + document.editor.textContainerOrigin.x, y: rect.midY + document.editor.textContainerOrigin.y), to: nil)
        let click = NSEvent.mouseEvent(with: .leftMouseDown, location: location, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, eventNumber: 1, clickCount: 1, pressure: 1)!
        document.editor.mouseDown(with: click)
        XCTAssertEqual(clicked, url)
    }
}
