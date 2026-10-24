import AppKit
import ImageIO
import UniformTypeIdentifiers
import XCTest
@testable import LorcaMac

func imageFixture(width: Int = 80, height: Int = 60) throws -> Data {
    let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
    context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    let output = NSMutableData()
    let destination = try XCTUnwrap(CGImageDestinationCreateWithData(output, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, try XCTUnwrap(context.makeImage()), nil)
    XCTAssertTrue(CGImageDestinationFinalize(destination))
    return output as Data
}

final class ImageTests: XCTestCase {
    private var directory: URL!
    private var storage: AttachmentStore!
    override func setUpWithError() throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        storage = AttachmentStore(directory: directory)
    }
    override func tearDownWithError() throws { try? FileManager.default.removeItem(at: directory) }

    func testValidImageIsNormalizedAndSurvivesOriginalDeletion() throws {
        let source = directory.appendingPathComponent("input.png")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try imageFixture().write(to: source)
        let attachment = try storage.importFile(source)
        try FileManager.default.removeItem(at: source)
        let data = try storage.data(for: attachment)
        XCTAssertEqual(attachment.width, 80)
        XCTAssertEqual(attachment.height, 60)
        XCTAssertTrue(attachment.fileName.hasSuffix(".jpg"))
        XCTAssertEqual(data.prefix(2), Data([0xff, 0xd8]))
        XCTAssertNotNil(storage.thumbnail(for: attachment))
        try storage.remove(attachment.id)
        XCTAssertThrowsError(try storage.data(for: attachment))
    }

    func testInvalidOversizedAndMissingImagesFailGracefully() throws {
        XCTAssertThrowsError(try storage.importData(Data("not an image".utf8), name: "fake.png"))
        XCTAssertThrowsError(try storage.importData(Data(count: AttachmentStore.maxSourceBytes + 1), name: "large.png"))
        let attachment = ChatAttachment(id: UUID(), name: "missing", width: 2, height: 2)
        XCTAssertNil(storage.thumbnail(for: attachment))
        XCTAssertThrowsError(try storage.data(for: attachment))
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.path))
    }

    func testLargeDimensionsAreBoundedAndNamesCannotEscapeStorage() throws {
        let attachment = try storage.importData(imageFixture(width: 5000, height: 20), name: "../../external.png")
        XCTAssertEqual(attachment.width, 4096)
        XCTAssertLessThanOrEqual(attachment.height, 20)
        XCTAssertEqual(storage.url(for: attachment).deletingLastPathComponent().path, directory.path)
        XCTAssertNoThrow(try storage.data(for: attachment))
    }

    func testClipboardAndDropProvidersLoadRealImageBytesAndFiles() async throws {
        let data = try imageFixture()
        let provider = NSItemProvider(item: data as NSData, typeIdentifier: UTType.png.identifier)
        let pasted = try await ImageInput.load(provider)
        XCTAssertNoThrow(try pasted.store(in: storage))
        let file = directory.appendingPathComponent("dropped.png")
        try data.write(to: file)
        let dropped = try await ImageInput.load(NSItemProvider(contentsOf: file)!)
        XCTAssertNoThrow(try dropped.store(in: storage))
    }
    func testNativeClipboardAcceptsMultipleImagesAndLeavesTextAlone() throws {
        let clipboard = NSPasteboard(name: NSPasteboard.Name("dev.lorca.tests.\(UUID())"))
        defer { clipboard.releaseGlobally() }
        let first = NSPasteboardItem()
        let second = NSPasteboardItem()
        first.setData(try imageFixture(), forType: .png)
        second.setData(try imageFixture(), forType: .png)
        clipboard.writeObjects([first, second])
        XCTAssertEqual(ClipboardImages.read(clipboard).count, 2)
        clipboard.clearContents()
        clipboard.setString("Normal text", forType: .string)
        XCTAssertTrue(ClipboardImages.read(clipboard).isEmpty)
    }

}
