import AppKit

// Compiled with ../Icons.swift by run.sh: a plain executable that exits non-zero on the first failed check.
@main
struct IconsTests {
    static func check(_ ok: Bool, _ what: String) {
        if !ok { print("FAIL: \(what)"); exit(1) }
    }

    static func main() {
        let dir = NSTemporaryDirectory() + "telmo-icons-test-\(getpid())"
        defer { try? FileManager.default.removeItem(atPath: dir) }

        // Reading an icon draws nothing on screen and launches nothing.
        let app = "/System/Applications/Calculator.app"
        let file = Icons.render(app: app, into: dir)
        check(file != nil, "renders the Calculator icon")
        if let file {
            check(file.hasPrefix(dir) && file.hasSuffix(".png"), "lands in the cache directory")
            let rep = NSBitmapImageRep(data: try! Data(contentsOf: URL(fileURLWithPath: file)))
            check(rep?.pixelsWide == Icons.pixels && rep?.pixelsHigh == Icons.pixels, "is 64 pixels square")
            check(Icons.render(app: app, into: dir) == file, "asks the cache the second time")
        }

        check(Icons.render(app: "/etc/passwd", into: dir) == nil, "refuses what is not an app")
        check(Icons.render(app: "System/Applications/Calculator.app", into: dir) == nil, "refuses a relative path")
        check(Icons.render(app: "/Applications/Nope.app", into: dir) == nil, "refuses a missing app")
        check(Icons.hash("a") == Icons.hash("a") && Icons.hash("a") != Icons.hash("b"), "hashes are stable and differ")
        check(Icons.isIconCommand("icon /A/B C.app") && !Icons.isIconCommand("toggle net"), "recognises the command")
        check(Icons.handle("icon /nowhere.app").hasPrefix("error"), "answers an error for a missing app")
        print("icons: ok")
    }
}
