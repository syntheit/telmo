import AppKit

private let accent = NSColor(srgbRed: 0x7a / 255, green: 0xa2 / 255, blue: 0xf7 / 255, alpha: 1) // Tokyo Night blue
private let green = NSColor(srgbRed: 0x9e / 255, green: 0xce / 255, blue: 0x6a / 255, alpha: 1)
private let red = NSColor(srgbRed: 0xf7 / 255, green: 0x76 / 255, blue: 0x8e / 255, alpha: 1)
private let barHeight: CGFloat = 2

/// Borderless window that may sit over the menu bar and the notch, and reports clicks.
private final class IslandWindow: NSPanel {
    var onClick: (() -> Void)?
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    override func mouseDown(with event: NSEvent) { onClick?() }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// The pill: icon, text and a progress bar on black.
private final class IslandView: NSView {
    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private let track = CALayer()
    private let fill = CALayer()
    private let shimmer = CAGradientLayer()
    private var shimmerWidth: CGFloat = -1
    private var content: IslandContent?
    var notched = true { didSet { updateCorners() } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        layer?.masksToBounds = true
        layer?.cornerCurve = .continuous
        icon.imageScaling = .scaleProportionallyDown
        label.font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium)
        label.textColor = NSColor(white: 0.92, alpha: 1)
        label.alignment = .center
        label.lineBreakMode = .byClipping
        addSubview(icon)
        addSubview(label)
        track.backgroundColor = NSColor(white: 1, alpha: 0.14).cgColor
        fill.masksToBounds = true
        shimmer.colors = [NSColor.clear.cgColor, NSColor(white: 1, alpha: 0.35).cgColor, NSColor.clear.cgColor]
        shimmer.startPoint = CGPoint(x: 0, y: 0.5)
        shimmer.endPoint = CGPoint(x: 1, y: 0.5)
        fill.addSublayer(shimmer)
        layer?.addSublayer(track)
        layer?.addSublayer(fill)
        updateCorners()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func updateCorners() {
        layer?.maskedCorners = notched ? [.layerMinXMinYCorner, .layerMaxXMinYCorner]
            : [.layerMinXMinYCorner, .layerMaxXMinYCorner, .layerMinXMaxYCorner, .layerMaxXMaxYCorner]
        needsLayout = true
    }

    func apply(_ content: IslandContent) {
        self.content = content
        let (symbol, color): (String, NSColor) = switch content.kind {
        case .running: ("hammer.fill", accent)
        case .ok: ("checkmark.circle.fill", green)
        case .failed: ("xmark.octagon.fill", red)
        }
        let config = NSImage.SymbolConfiguration(pointSize: 13, weight: .semibold)
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)?.withSymbolConfiguration(config)
        icon.contentTintColor = color
        label.stringValue = content.text
        fill.backgroundColor = color.withAlphaComponent(content.fraction == nil ? 0.35 : 1).cgColor
        track.isHidden = content.kind == .failed
        fill.isHidden = content.kind == .failed
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let w = bounds.width, h = bounds.height
        let radius = notched ? 14 : h / 2
        layer?.cornerRadius = radius
        let wing = notched ? IslandGeometry.wing : w / 2
        // Notched: one item centred in each wing beside the notch. Capsule: icon left, text right.
        let iconSize: CGFloat = 18
        let centerY = notched ? h / 2 : h / 2 + barHeight / 2
        if notched {
            icon.frame = NSRect(x: (wing - iconSize) / 2, y: centerY - iconSize / 2, width: iconSize, height: iconSize)
            label.frame = NSRect(x: w - wing, y: centerY - 8, width: wing, height: 16)
        } else {
            icon.frame = NSRect(x: 16, y: centerY - iconSize / 2, width: iconSize, height: iconSize)
            label.frame = NSRect(x: 40, y: centerY - 8, width: w - 40 - 14, height: 16)
        }
        let inset = radius
        let trackWidth = max(0, w - 2 * inset)
        track.frame = CGRect(x: inset, y: 0, width: trackWidth, height: barHeight)
        let fraction = content?.fraction ?? 1
        fill.frame = CGRect(x: inset, y: 0, width: trackWidth * fraction, height: barHeight)
        shimmer.frame = CGRect(x: -40, y: 0, width: 40, height: barHeight)
        updateShimmer(width: fill.frame.width)
    }

    private func updateShimmer(width: CGFloat) {
        guard content?.kind == .running, !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion, width > 0 else {
            shimmer.removeAnimation(forKey: "shimmer")
            shimmerWidth = -1
            return
        }
        if abs(width - shimmerWidth) < 1 { return }
        shimmerWidth = width
        let sweep = CABasicAnimation(keyPath: "position.x")
        sweep.fromValue = -20
        sweep.toValue = width + 20
        sweep.duration = 2.4
        sweep.repeatCount = .infinity
        sweep.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        shimmer.add(sweep, forKey: "shimmer")
    }
}

/// A frozen picture of the popup that shrinks into the notch.
private final class SnapshotWindow: NSPanel {
    override var canBecomeKey: Bool { false }
}

/// The rebuild "notch pill": a Dynamic Island style readout of `rebuild.json` for when the System popup is closed.
final class RebuildIsland {
    var onClick: (() -> Void)?
    /// True while the System popup is open; it shows the progress itself.
    var isPopupOpen: () -> Bool = { false }

    private let window: IslandWindow
    private let view = IslandView(frame: .zero)
    private var tracker = IslandTracker()
    private var status: RebuildStatus?
    private var timer: Timer?
    private var visible = false
    private var suppressed = false
    private var epoch = 0 // invalidates the completion of an animation that was overtaken
    private var duration: TimeInterval { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 0.35 }

    init() {
        window = IslandWindow(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = false
        window.level = .statusBar
        window.hidesOnDeactivate = false
        window.isReleasedWhenClosed = false
        window.animationBehavior = .none
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        window.contentView = view
        window.onClick = { [weak self] in self?.clicked() }
    }

    func start() {
        let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in self?.tick() }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
        tick()
    }

    /// Whether the last status read says a rebuild is in progress.
    func isRebuilding() -> Bool {
        readStatus()
        return status?.state == .running
    }

    // MARK: Polling

    private func readStatus() {
        let path = RebuildStatus.path()
        status = FileManager.default.contents(atPath: path).flatMap(RebuildStatus.decode)
    }

    /// Reads the file and shows, updates or retracts the pill. Also run before the popup closes, so a finish seen
    /// under the popup counts as seen.
    func tick() {
        readStatus()
        guard !suppressed else { return }
        let popupOpen = isPopupOpen()
        if let content = tracker.update(status, popupOpen: popupOpen, now: Date()) {
            show(content)
        } else if visible {
            hide(animated: !popupOpen)
        }
    }

    private func clicked() {
        tracker.acknowledge(status)
        hide(animated: false)
        onClick?()
    }

    // MARK: Geometry

    private func targetScreen() -> NSScreen? {
        NSScreen.screens.first { $0.safeAreaInsets.top > 0 } ?? NSScreen.screens.first
    }

    private func notchRect(_ screen: NSScreen) -> CGRect? {
        IslandGeometry.notchRect(screen: screen.frame, topInset: screen.safeAreaInsets.top,
                                 leftArea: screen.auxiliaryTopLeftArea, rightArea: screen.auxiliaryTopRightArea)
    }

    private func frames(_ screen: NSScreen) -> (pill: CGRect, collapsed: CGRect) {
        let notch = notchRect(screen)
        let pill = IslandGeometry.pillFrame(screen: notch == nil ? screen.visibleFrame : screen.frame, notch: notch)
        let collapsed = notch.map { CGRect(x: $0.minX, y: pill.minY, width: $0.width, height: pill.height) }
            ?? CGRect(x: pill.midX - 30, y: pill.minY, width: 60, height: pill.height)
        return (pill, collapsed)
    }

    /// Where a minimizing popup should shrink to.
    func collapsedFrame() -> CGRect? {
        targetScreen().map { frames($0).collapsed }
    }

    // MARK: Show / hide

    private func show(_ content: IslandContent) {
        guard let screen = targetScreen() else { return }
        let (pill, collapsed) = frames(screen)
        view.notched = notchRect(screen) != nil
        view.apply(content)
        if visible {
            if window.frame != pill { window.setFrame(pill, display: true) }
            return
        }
        visible = true
        epoch += 1
        window.setFrame(collapsed, display: true)
        view.alphaValue = 0
        window.alphaValue = 1
        window.orderFrontRegardless()
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = duration
            ctx.timingFunction = CAMediaTimingFunction(name: .easeOut)
            window.animator().setFrame(pill, display: true)
        }
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = duration * 0.6
            view.animator().alphaValue = 1
        }
    }

    private func hide(animated: Bool) {
        guard visible else { return }
        visible = false
        epoch += 1
        let mine = epoch
        guard animated, duration > 0, let screen = targetScreen() else {
            window.orderOut(nil)
            return
        }
        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = CAMediaTimingFunction(name: .easeIn)
            window.animator().setFrame(frames(screen).collapsed, display: true)
            view.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            if let self, mine == epoch { window.orderOut(nil) }
        })
    }

    // MARK: Minimize

    /// Shrinks a picture of the closing popup into the notch, then lets the pill grow out of it.
    func minimize(image: NSImage, from frame: CGRect, to target: CGRect) {
        suppressed = true
        let ghost = SnapshotWindow(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        ghost.isOpaque = false
        ghost.backgroundColor = .clear
        ghost.hasShadow = false
        ghost.level = .popUpMenu
        ghost.isReleasedWhenClosed = false
        ghost.animationBehavior = .none
        ghost.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        let picture = NSImageView(frame: NSRect(origin: .zero, size: frame.size))
        picture.image = image
        picture.imageScaling = .scaleAxesIndependently
        picture.autoresizingMask = [.width, .height]
        picture.wantsLayer = true
        picture.layer?.cornerRadius = 24
        picture.layer?.cornerCurve = .continuous
        picture.layer?.masksToBounds = true
        picture.layer?.backgroundColor = NSColor(white: 0.04, alpha: 0.92).cgColor
        ghost.contentView = picture
        ghost.orderFrontRegardless()

        NSAnimationContext.runAnimationGroup({ ctx in
            ctx.duration = duration
            ctx.timingFunction = CAMediaTimingFunction(name: .easeIn)
            ghost.animator().setFrame(target, display: true)
        }, completionHandler: { [weak self] in
            ghost.orderOut(nil)
            self?.suppressed = false
            self?.tick()
        })
        DispatchQueue.main.asyncAfter(deadline: .now() + duration * 0.7) {
            NSAnimationContext.runAnimationGroup { ctx in
                ctx.duration = self.duration * 0.3
                ghost.animator().alphaValue = 0
            }
        }
    }

    /// Minimizing is skipped when Reduce Motion is on.
    var canAnimate: Bool { duration > 0 }
}
