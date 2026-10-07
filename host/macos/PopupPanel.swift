import AppKit

private let normalCols = 90
private let normalRows = 22
private let largeFraction: CGFloat = 0.8
private let padding: CGFloat = 14

final class PopupPanel: NSPanel, LocalProcessTerminalViewDelegate {
    var onExit: (() -> Void)?
    var onEscape: (() -> Void)?
    private var cols = normalCols
    private var rows = normalRows
    private var escapeMonitor: Any?
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
        configureTerminal(terminal, cols: cols, rows: rows)
        contentView = makeBackground()
        holder.autoresizingMask = [.width, .height]
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    // MARK: Setup

    private func configureTerminal(_ terminal: LocalProcessTerminalView, cols: Int, rows: Int) {
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
        tint.autoresizingMask = [.width, .height]
        tint.wantsLayer = true
        tint.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.8).cgColor
        blur.addSubview(tint)
        blur.addSubview(holder)
        return blur
    }

    // MARK: Lifecycle

    func run(_ launch: Launch) {
        terminateChild()
        removeEscapeMonitor()
        chooseGrid(large: launch.large)
        replaceTerminal()
        setContentSize(NSSize(width: terminal.frame.width + 2 * padding, height: terminal.frame.height + 2 * padding))
        if launch.escapeCloses { installEscapeMonitor() }
        isRunning = true
        var env = ProcessInfo.processInfo.environment
        env["TERM"] = "xterm-256color"
        env["COLORTERM"] = "truecolor"
        env["TELMO_HOST"] = "1"
        env["PATH"] = ModuleLookup.searchPath.joined(separator: ":")
        terminal.startProcess(executable: launch.executable, args: launch.args, environment: env.map { "\($0.key)=\($0.value)" })
    }

    /// Esc closes the popup for TUIs whose own Esc does something else (btop opens its menu).
    private func installEscapeMonitor() {
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, event.window === self, event.keyCode == 53 else { return event }
            onEscape?()
            return nil
        }
    }

    private func removeEscapeMonitor() {
        if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
        escapeMonitor = nil
    }

    /// 90×22 cells, or 80% of the mouse's screen rounded down to whole cells.
    private func chooseGrid(large: Bool) {
        guard large, let area = targetScreen()?.visibleFrame else {
            (cols, rows) = (normalCols, normalRows)
            return
        }
        let cell = cellSize()
        cols = max(normalCols, Int((area.width * largeFraction - 2 * padding) / cell.width))
        rows = max(normalRows, Int((area.height * largeFraction - 2 * padding) / cell.height))
    }

    /// Cell size in points, from how the fitted frame grows with one more column and row.
    private func cellSize() -> NSSize {
        let probe = LocalProcessTerminalView(frame: .zero)
        configureTerminal(probe, cols: normalCols, rows: normalRows)
        let small = probe.getOptimalFrameSize().size
        probe.getTerminal().resize(cols: normalCols + 1, rows: normalRows + 1)
        let big = probe.getOptimalFrameSize().size
        return NSSize(width: big.width - small.width, height: big.height - small.height)
    }

    private func targetScreen() -> NSScreen? {
        let point = NSEvent.mouseLocation
        return NSScreen.screens.first { $0.frame.contains(point) } ?? NSScreen.main
    }

    private func replaceTerminal() {
        let old = terminal
        let fresh = LocalProcessTerminalView(frame: old.frame)
        configureTerminal(fresh, cols: cols, rows: rows)
        old.processDelegate = nil
        old.removeFromSuperview()
        holder.addSubview(fresh)
        terminal = fresh
    }

    func show() {
        centerOnScreen()
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

    private func centerOnScreen() {
        guard let area = targetScreen()?.visibleFrame else { return }
        setFrameOrigin(NSPoint(x: (area.midX - frame.width / 2).rounded(), y: (area.midY - frame.height / 2).rounded()))
    }

    func hide() {
        isRunning = false
        removeEscapeMonitor()
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
