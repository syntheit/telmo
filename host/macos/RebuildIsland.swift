import AppKit

private func color(_ rgb: UInt32) -> CGColor {
    CGColor(srgbRed: CGFloat(rgb >> 16 & 0xff) / 255, green: CGFloat(rgb >> 8 & 0xff) / 255, blue: CGFloat(rgb & 0xff) / 255, alpha: 1)
}

/// Borderless window that may sit over the menu bar and the notch, and reports clicks.
private final class IslandWindow: NSPanel {
    var onClick: (() -> Void)?
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
    override func mouseDown(with event: NSEvent) { onClick?() }
    override func constrainFrameRect(_ frameRect: NSRect, to screen: NSScreen?) -> NSRect { frameRect }
}

/// The Nix snowflake: six lambdas, alternately deep and light blue.
private enum Snowflake {
    static let size: CGFloat = 20
    private static let deep: UInt32 = 0x5277c3
    private static let light: UInt32 = 0x7ebae4

    /// One lambda of the official `nix-snowflake` artwork, in its SVG coordinates (y down).
    private static let start = CGPoint(x: 309.54892, y: -710.38827)
    private static let steps: [(CGFloat, CGFloat)] = [
        (122.19683, 211.67512), (-56.15706, 0.5268), (-32.6236, -56.8692), (-32.85645, 56.5653),
        (-27.90237, -0.011), (-14.29086, -24.6896), (46.81047, -80.4901), (-33.22946, -57.8257),
    ]
    /// The point the six lambdas turn around, and the width of the artwork.
    private static let center = CGPoint(x: 407.3, y: -715.8)
    private static let artwork: CGFloat = 501.5625

    /// The layers, centred on the origin of their parent. The first lambda is deep, then they alternate.
    static func layers() -> [CAShapeLayer] {
        let scale = size / artwork
        // To points around the centre, with y up like AppKit.
        func place(_ p: CGPoint) -> CGPoint { CGPoint(x: (p.x - center.x) * scale, y: -(p.y - center.y) * scale) }
        let path = CGMutablePath()
        var point = start
        path.move(to: place(point))
        for (dx, dy) in steps {
            point = CGPoint(x: point.x + dx, y: point.y + dy)
            path.addLine(to: place(point))
        }
        path.closeSubpath()
        return (0..<6).map { i in
            let layer = CAShapeLayer()
            var turn = CGAffineTransform(rotationAngle: CGFloat(i) * .pi / 3)
            layer.path = path.copy(using: &turn)
            layer.fillColor = color(i % 2 == 0 ? deep : light)
            layer.fillRule = .evenOdd
            return layer
        }
    }
}

/// The pill: the Nix snowflake in the left wing, ten bars of progress in the right.
private final class IslandView: NSView {
    private static let dim: Float = 0.16
    private let logo = CALayer()
    private let bars: [CALayer] = (0..<IslandContent.bars).map { _ in
        let bar = CALayer()
        bar.bounds = CGRect(x: 0, y: 0, width: IslandView.barWidth, height: IslandView.barHeight)
        bar.cornerRadius = IslandView.barWidth / 2
        bar.opacity = IslandView.dim
        return bar
    }
    private static let barWidth: CGFloat = 2.4
    private static let barHeight: CGFloat = 12
    private static let pitch: CGFloat = 5
    /// What the bars show now; nil until drawn, and again after the pill was hidden (animations don't survive that).
    private var shown: (content: IslandContent, still: Bool)?
    var notched = true { didSet { updateCorners() } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        layer?.masksToBounds = true
        layer?.cornerCurve = .continuous
        Snowflake.layers().forEach(logo.addSublayer)
        layer?.addSublayer(logo)
        bars.forEach { layer?.addSublayer($0) }
        updateCorners()
    }

    required init?(coder: NSCoder) { fatalError() }

    private func updateCorners() {
        layer?.maskedCorners = notched ? [.layerMinXMinYCorner, .layerMaxXMinYCorner]
            : [.layerMinXMinYCorner, .layerMaxXMinYCorner, .layerMinXMaxYCorner, .layerMaxXMaxYCorner]
        needsLayout = true
    }

    /// Forget what is drawn, so the next `apply` starts the animations again.
    func invalidate() { shown = nil }

    func apply(_ content: IslandContent) {
        let still = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        if let shown, shown.content == content, shown.still == still { return }
        // The first drawing snaps; later changes cross-fade (0.25 s, Core Animation's default).
        let fade = shown != nil
        shown = (content, still)
        CATransaction.begin()
        CATransaction.setDisableActions(!fade)
        let tint = color(content.look.rgb)
        for (i, bar) in bars.enumerated() {
            bar.removeAllAnimations()
            bar.backgroundColor = tint
            bar.opacity = i < content.lit ? 1 : Self.dim
            if content.sweeping {
                if still { bar.opacity = 0.4 } else { bar.add(sweep(bar: i), forKey: "sweep") }
            } else if i == content.head {
                if still { bar.opacity = 0.5 } else { bar.add(breath(), forKey: "breath") }
            }
        }
        CATransaction.commit()
    }

    /// The head pulses 0.22 to 0.80 along a sine, 1.8 s a round.
    private func breath() -> CABasicAnimation {
        let a = CABasicAnimation(keyPath: "opacity")
        a.fromValue = 0.22
        a.toValue = 0.80
        a.duration = 0.9
        a.autoreverses = true
        a.repeatCount = .infinity
        a.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        return a
    }

    /// A soft highlight runs over the bars and back, 3.2 s a round.
    private func sweep(bar: Int) -> CAKeyframeAnimation {
        let steps = 64
        let a = CAKeyframeAnimation(keyPath: "opacity")
        a.values = (0...steps).map { step -> Double in
            let phase = 2 * Double.pi * Double(step) / Double(steps)
            let pos = Double(IslandContent.bars - 1) * (1 - cos(phase)) / 2
            return 0.16 + 0.72 * max(0, 1 - abs(Double(bar) - pos) / 1.6)
        }
        a.duration = 3.2
        a.repeatCount = .infinity
        return a
    }

    override func layout() {
        super.layout()
        let w = bounds.width, h = bounds.height
        let radius = notched ? 14 : h / 2
        layer?.cornerRadius = radius
        // One item centred in each wing: beside the notch, or in either half of the capsule.
        let wing = notched ? IslandGeometry.wing : w / 2
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        logo.position = CGPoint(x: wing / 2, y: h / 2)
        let span = CGFloat(bars.count - 1) * Self.pitch
        let first = w - wing / 2 - span / 2
        for (i, bar) in bars.enumerated() {
            bar.position = CGPoint(x: first + CGFloat(i) * Self.pitch, y: h / 2)
        }
        CATransaction.commit()
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
    /// Minimize animations in flight; the pill waits for the last one.
    private var minimizing = 0
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
        status = FileManager.default.contents(atPath: path).flatMap(RebuildStatus.decode)?.checked()
    }

    /// Reads the file and shows, updates or retracts the pill. Also run before the popup closes, so a finish seen
    /// under the popup counts as seen.
    func tick() {
        readStatus()
        let popupOpen = isPopupOpen()
        // Track even while minimizing, so a finish the popup showed stays seen.
        let content = tracker.update(status, popupOpen: popupOpen, now: Date())
        guard minimizing == 0 else { return }
        if let content {
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
        if !visible { view.invalidate() }
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
        minimizing += 1
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
            guard let self else { return }
            minimizing -= 1
            tick()
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
