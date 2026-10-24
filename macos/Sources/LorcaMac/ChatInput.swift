import AppKit
import SwiftUI

struct ChatInput: NSViewRepresentable {
    @Binding var text: String
    var size: CGFloat
    var maxLines: Int
    var identifier: String
    var send: () -> Void
    var attach: ([ImageInput]) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSScrollView {
        let editor = Editor()
        editor.isRichText = false
        editor.importsGraphics = false
        editor.drawsBackground = false
        editor.textContainerInset = .zero
        editor.textContainer?.lineFragmentPadding = 0
        editor.isVerticallyResizable = true
        editor.isHorizontallyResizable = false
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        editor.textColor = .labelColor
        editor.insertionPointColor = .controlAccentColor
        editor.delegate = context.coordinator
        editor.registerForDraggedTypes([.fileURL, .png, .tiff])
        editor.setAccessibilityIdentifier(identifier)
        editor.setAccessibilityLabel("Message lorca…")
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.documentView = editor
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let editor = scroll.documentView as? Editor else { return }
        editor.send = send
        editor.attach = attach
        editor.font = .systemFont(ofSize: size)
        if editor.string != text { editor.string = text }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView scroll: NSScrollView, context: Context) -> CGSize? {
        guard let editor = scroll.documentView as? Editor else { return nil }
        let width = max(1, proposal.width ?? 300)
        editor.textContainer?.containerSize = NSSize(width: width, height: .greatestFiniteMagnitude)
        if let container = editor.textContainer { editor.layoutManager?.ensureLayout(for: container) }
        let height = editor.textContainer.flatMap { editor.layoutManager?.usedRect(for: $0).height } ?? size * 1.3
        return CGSize(width: width, height: min(size * 1.4 * CGFloat(maxLines), max(size * 1.3, height)))
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: ChatInput
        init(_ parent: ChatInput) { self.parent = parent }
        func textDidChange(_ notification: Notification) {
            if let editor = notification.object as? NSTextView { parent.text = editor.string }
        }
    }

    final class Editor: NSTextView {
        var send: (() -> Void)?
        var attach: (([ImageInput]) -> Void)?
        override func keyDown(with event: NSEvent) {
            if [36, 76].contains(event.keyCode), !event.modifierFlags.contains(.shift) { send?() }
            else { super.keyDown(with: event) }
        }
        override func paste(_ sender: Any?) {
            let images = ClipboardImages.read(.general)
            if images.isEmpty { super.paste(sender) }
            else { attach?(images) }
        }
        override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
            if item.action == #selector(paste(_:)), !ClipboardImages.read(.general).isEmpty { return true }
            return super.validateUserInterfaceItem(item)
        }
        override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
            ClipboardImages.read(sender.draggingPasteboard).isEmpty ? super.draggingEntered(sender) : .copy
        }
        override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
            let images = ClipboardImages.read(sender.draggingPasteboard)
            guard !images.isEmpty else { return super.performDragOperation(sender) }
            attach?(images)
            return true
        }
    }
}
