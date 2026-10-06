import AppKit

private let cols = 90
private let rows = 22
private let padding: CGFloat = 14

final class PopupPanel: NSPanel, LocalProcessTerminalViewDelegate {
    var onExit: (() -> Void)?
    // Every child gets its own terminal view, so a late callback from a replaced
    // child (source !== terminal) is recognisably stale.
    private var terminal = LocalProcessTerminalView(frame: .zero)
    private let holder = NSView()
    private(set) var isRunning = false

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        level = .popUpMenu
        collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary, .ignoresCycle]
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = false
        appearance = NSAppearance(named: .darkAqua)
        animationBehavior = .none
        configureTerminal(terminal)
        contentView = makeBackground()
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    // MARK: Setup

    private func configureTerminal(_ terminal: LocalProcessTerminalView) {
        let size: CGFloat = 13
        terminal.font = NSFont(name: "JetBrainsMono Nerd Font Mono", size: size)
            ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
        terminal.nativeBackgroundColor = .black
        terminal.backgroundOpacity = 0 // the container paints the background
        terminal.nativeForegroundColor = NSColor(white: 0.92, alpha: 1)
        terminal.processDelegate = self
        terminal.getTerminal().resize(cols: cols, rows: rows)
        let fit = terminal.getOptimalFrameSize().size
        terminal.frame = NSRect(x: padding, y: padding, width: fit.width.rounded(.up), height: fit.height.rounded(.up))
        for case let scroller as NSScroller in terminal.subviews { scroller.isHidden = true }
    }

    private func makeBackground() -> NSView {
        let radius: CGFloat = 24
        let size = NSSize(width: terminal.frame.width + 2 * padding, height: terminal.frame.height + 2 * padding)
        setContentSize(size)
        let bounds = NSRect(origin: .zero, size: size)
        holder.frame = bounds
        holder.addSubview(terminal)

        let blur = NSVisualEffectView(frame: bounds)
        blur.material = .hudWindow
        blur.blendingMode = .behindWindow
        blur.state = .active
        blur.appearance = NSAppearance(named: .darkAqua)
        blur.wantsLayer = true
        blur.layer?.cornerRadius = radius
        blur.layer?.cornerCurve = .continuous
        blur.layer?.masksToBounds = true
        let tint = NSView(frame: bounds)
        tint.wantsLayer = true
        tint.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.8).cgColor
        blur.addSubview(tint)
        blur.addSubview(holder)
        return blur
    }

    // MARK: Lifecycle

    func run(executable: String) {
        terminateChild()
        replaceTerminal()
        isRunning = true
        var env = ProcessInfo.processInfo.environment
        env["TERM"] = "xterm-256color"
        env["COLORTERM"] = "truecolor"
        env["TELMO_HOST"] = "1"
        env["PATH"] = ModuleLookup.searchPath.joined(separator: ":")
        terminal.startProcess(executable: executable, environment: env.map { "\($0.key)=\($0.value)" })
    }

    private func replaceTerminal() {
        let old = terminal
        let fresh = LocalProcessTerminalView(frame: old.frame)
        configureTerminal(fresh)
        old.processDelegate = nil
        old.removeFromSuperview()
        holder.addSubview(fresh)
        terminal = fresh
    }

    func show() {
        center(on: NSEvent.mouseLocation)
        makeKeyAndOrderFront(nil)
        makeFirstResponder(terminal)
        invalidateShadow()
    }

    func terminateChild() {
        guard isRunning else { return }
        isRunning = false
        terminal.terminate()
    }

    /// The visible text of the terminal, one line per row.
    func visibleText() -> String {
        let term = terminal.getTerminal()
        return (0..<term.rows)
            .map { term.getLine(row: $0)?.translateToString(trimRight: true) ?? "" }
            .joined(separator: "\n")
            .replacingOccurrences(of: "\0", with: " ") // empty cells
    }

    private func center(on point: NSPoint) {
        let screen = NSScreen.screens.first { $0.frame.contains(point) } ?? NSScreen.main
        guard let area = screen?.visibleFrame else { return }
        setFrameOrigin(NSPoint(x: (area.midX - frame.width / 2).rounded(), y: (area.midY - frame.height / 2).rounded()))
    }

    func hide() {
        isRunning = false
        orderOut(nil)
    }

    // MARK: LocalProcessTerminalViewDelegate

    func processTerminated(source: TerminalView, exitCode: Int32?) {
        DispatchQueue.main.async { [self] in
            guard source === terminal, isRunning else { return }
            isRunning = false
            onExit?()
        }
    }

    func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}
    func setTerminalTitle(source: LocalProcessTerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
}
