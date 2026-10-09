import AppKit

/// Polls the general pasteboard and hands every change to `telmo-clipboard ingest`, one at a time.
final class ClipboardWatcher {
    static var configPath: String { NSHomeDirectory() + "/.config/telmo/clipboard.json" }
    static var isEnabled: Bool { FileManager.default.fileExists(atPath: configPath) }

    private let pasteboard = NSPasteboard.general
    private var lastCount: Int
    private var timer: Timer?
    private let queue = DispatchQueue(label: "telmo.clipboard.ingest")
    private var warned = false // touched only on `queue`

    init() { lastCount = pasteboard.changeCount }

    func start() {
        let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in self?.poll() }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private func poll() {
        let count = pasteboard.changeCount
        guard count != lastCount else { return }
        lastCount = count
        let source = NSWorkspace.shared.frontmostApplication?.localizedName ?? ""
        guard let item = ClipboardItem.read(pasteboard), let file = try? item.writeTemp() else { return }
        queue.async { [self] in ingest(kind: item.kind, file: file, source: source) }
    }

    private func ingest(kind: String, file: URL, source: String) {
        guard let exe = ModuleLookup.find("clipboard") else {
            fail(file, "telmo-clipboard not found")
            return
        }
        let task = Process()
        task.executableURL = URL(fileURLWithPath: exe)
        task.arguments = ["ingest", "--kind", kind, "--file", file.path, "--source", source]
        do {
            try task.run()
            task.waitUntilExit()
        } catch {
            fail(file, "cannot run telmo-clipboard: \(error)")
        }
    }

    private func fail(_ file: URL, _ message: String) {
        try? FileManager.default.removeItem(at: file)
        if !warned { NSLog("telmo: clipboard: \(message)") }
        warned = true
    }
}
