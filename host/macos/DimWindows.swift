import AppKit

private final class DimWindow: NSWindow {
    var onClick: (() -> Void)?
    override var canBecomeKey: Bool { false }
    override func mouseDown(with event: NSEvent) { onClick?() }
}

/// One black click-catching window per screen, created once and reused.
final class DimWindows {
    var onClick: (() -> Void)? { didSet { windows.values.forEach { $0.onClick = onClick } } }
    private var windows: [CGDirectDisplayID: DimWindow] = [:]
    private let alpha: CGFloat = 0.35
    private var fade: TimeInterval { NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 0.12 }

    func show(below panel: NSWindow) {
        for screen in NSScreen.screens {
            let window = window(for: screen)
            window.setFrame(screen.frame, display: false)
            window.level = NSWindow.Level(rawValue: panel.level.rawValue - 1)
            if window.isVisible { continue }
            window.alphaValue = 0
            window.orderFrontRegardless()
            NSAnimationContext.runAnimationGroup {
                $0.duration = fade
                window.animator().alphaValue = 1
            }
        }
    }

    func hide() {
        for window in windows.values where window.isVisible {
            NSAnimationContext.runAnimationGroup({
                $0.duration = fade
                window.animator().alphaValue = 0
            }, completionHandler: {
                // A new popup may have faded the window back in meanwhile.
                if window.alphaValue == 0 { window.orderOut(nil) }
            })
        }
    }

    private func window(for screen: NSScreen) -> DimWindow {
        let id = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID ?? 0
        if let existing = windows[id] { return existing }
        let window = DimWindow(contentRect: screen.frame, styleMask: [.borderless], backing: .buffered, defer: false)
        window.backgroundColor = NSColor.black.withAlphaComponent(alpha)
        window.isOpaque = false
        window.hasShadow = false
        window.isReleasedWhenClosed = false
        window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary, .ignoresCycle]
        window.onClick = onClick
        windows[id] = window
        return window
    }
}
