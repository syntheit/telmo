import AppKit

/// App icons for the launcher: `icon <path to an .app>` replies with the path of a small PNG of the icon the Finder shows.
/// The popup's process can't ask AppKit itself, so it asks us. Rendered files are cached by bundle path and Info.plist mtime.
enum Icons {
    static let pixels = 64

    static func isIconCommand(_ line: String) -> Bool { line.hasPrefix("icon ") }

    static var cacheDir: String {
        let base = ProcessInfo.processInfo.environment["XDG_CACHE_HOME"].flatMap { $0.isEmpty ? nil : $0 }
            ?? NSHomeDirectory() + "/.cache"
        return base + "/telmo/icons"
    }

    /// `icon <path>`: the path may hold spaces, so everything after the first space is the path.
    static func handle(_ line: String) -> String {
        let path = String(line.dropFirst("icon ".count))
        guard let file = render(app: path, into: cacheDir) else { return "error no icon for \(path)" }
        return file
    }

    /// FNV-1a: stable across runs, unlike Swift's randomly seeded hashValue.
    static func hash(_ text: String) -> String {
        var h: UInt64 = 0xcbf29ce484222325
        for byte in text.utf8 { h = (h ^ UInt64(byte)) &* 0x100000001b3 }
        return String(h, radix: 16)
    }

    /// Changes when the app is updated, so a new icon is drawn.
    static func cacheKey(app: String) -> String {
        let plist = app + "/Contents/Info.plist"
        let attrs = (try? FileManager.default.attributesOfItem(atPath: plist))
            ?? (try? FileManager.default.attributesOfItem(atPath: app)) ?? [:]
        let stamp = (attrs[.modificationDate] as? Date)?.timeIntervalSince1970 ?? 0
        return hash("\(app)|\(stamp)|\(pixels)")
    }

    /// The PNG for an app bundle, drawing it first if needed. Only `.app` paths that exist qualify.
    static func render(app: String, into dir: String) -> String? {
        var isDir: ObjCBool = false
        guard app.hasSuffix(".app"), FileManager.default.fileExists(atPath: app, isDirectory: &isDir), isDir.boolValue else { return nil }
        let target = dir + "/" + cacheKey(app: app) + ".png"
        if FileManager.default.fileExists(atPath: target) { return target }
        guard let png = pngData(of: NSWorkspace.shared.icon(forFile: app)) else { return nil }
        do {
            try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            let temp = target + ".\(getpid()).tmp"
            try png.write(to: URL(fileURLWithPath: temp))
            rename(temp, target)
        } catch { return nil }
        return target
    }

    static func pngData(of icon: NSImage) -> Data? {
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4,
            hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
              let context = NSGraphicsContext(bitmapImageRep: rep) else { return nil }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        context.imageInterpolation = .high
        icon.draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels), from: .zero, operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        return rep.representation(using: .png, properties: [:])
    }
}
