// Run with tests/run.sh. Covers ClipboardItem.read on a private pasteboard (never the general one).
import AppKit

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

@main struct ClipboardTests {
    static func main() {
        let pb = NSPasteboard(name: NSPasteboard.Name("telmo.test.\(UUID().uuidString)"))
        defer { pb.releaseGlobally() }

        pb.clearContents()
        check(ClipboardItem.read(pb) == nil, "empty pasteboard")

        pb.setString("hello", forType: .string)
        let text = ClipboardItem.read(pb)
        check(text?.kind == "text" && text?.data == Data("hello".utf8), "text")

        pb.clearContents()
        pb.setString(String(repeating: "a", count: ClipboardItem.maxTextBytes + 1), forType: .string)
        check(ClipboardItem.read(pb) == nil, "oversized text skipped")

        pb.clearContents()
        let image = NSImage(size: NSSize(width: 4, height: 4), flipped: false) { rect in
            NSColor.red.setFill(); rect.fill(); return true
        }
        pb.writeObjects([image])
        let png = ClipboardItem.read(pb)
        check(png?.kind == "image" && png?.data.starts(with: [0x89, 0x50, 0x4E, 0x47]) == true, "image converted to png")

        pb.clearContents()
        pb.writeObjects([URL(fileURLWithPath: "/tmp/a b") as NSURL, URL(fileURLWithPath: "/tmp/c") as NSURL])
        pb.setString("ignored", forType: .string)
        let files = ClipboardItem.read(pb)
        check(files?.kind == "files" && files.map { String(decoding: $0.data, as: UTF8.self) } == "/tmp/a b\n/tmp/c", "files win, newline separated")

        if let item = text, let url = try? item.writeTemp() {
            check((try? Data(contentsOf: url)) == item.data, "temp file holds the data")
            try? FileManager.default.removeItem(at: url)
        } else { check(false, "temp file written") }
        print("ok")
    }
}
