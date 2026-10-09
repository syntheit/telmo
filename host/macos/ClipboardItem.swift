import AppKit

/// What a pasteboard change looks like to `telmo-clipboard ingest`.
struct ClipboardItem {
    let kind: String // text, image or files
    let data: Data

    static let maxImageBytes = 30 << 20
    static let maxTextBytes = 1 << 20

    /// Best representation: file URLs, then an image (as PNG), then a string. Nil for anything else or too large.
    static func read(_ pasteboard: NSPasteboard) -> ClipboardItem? {
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            return ClipboardItem(kind: "files", data: Data(urls.map(\.path).joined(separator: "\n").utf8))
        }
        if let png = png(from: pasteboard) {
            return png.count <= maxImageBytes ? ClipboardItem(kind: "image", data: png) : nil
        }
        if let text = pasteboard.string(forType: .string) {
            let data = Data(text.utf8)
            return data.count <= maxTextBytes ? ClipboardItem(kind: "text", data: data) : nil
        }
        return nil
    }

    private static func png(from pasteboard: NSPasteboard) -> Data? {
        if let png = pasteboard.data(forType: .png) { return png }
        guard let tiff = pasteboard.data(forType: .tiff) else { return nil }
        return NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
    }

    /// Writes the data to a unique temp file for the ingester, which deletes it.
    func writeTemp() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("telmo-clip-\(UUID().uuidString)")
        try data.write(to: url, options: .atomic)
        return url
    }
}
