import AppKit
import CoreLocation

final class AppDelegate: NSObject, NSApplicationDelegate, CLLocationManagerDelegate {
    private let popup = PopupPanel()
    private let dim = DimWindows()
    private var hotkeys: Hotkeys?
    private var ipc: IPCServer?
    private var locationManager: CLLocationManager?
    private var previousApp: NSRunningApplication?
    private var module: String?
    private var signalSources: [DispatchSourceSignal] = []

    func applicationDidFinishLaunching(_ note: Notification) {
        if IPCServer.isRunning() { exit(0) }
        popup.onExit = { [weak self] in self?.close() }
        dim.onClick = { [weak self] in self?.hide() }

        let server = IPCServer { [weak self] line in self?.handle(line) ?? "error shutting down" }
        do { try server.start() } catch {
            NSLog("telmo: cannot listen on socket: \(error)")
            exit(1)
        }
        ipc = server

        for sig in [SIGTERM, SIGINT] {
            signal(sig) { _ in } // not SIG_IGN: children would inherit the ignore
            let source = DispatchSource.makeSignalSource(signal: sig, queue: .main)
            source.setEventHandler { NSApp.terminate(nil) }
            source.resume()
            signalSources.append(source)
        }

        hotkeys = Hotkeys { [weak self] module in self?.toggle(module) }
    }

    func applicationWillTerminate(_ note: Notification) {
        popup.terminateChild() // otherwise the telmo-* child outlives the host
    }

    // MARK: Commands (main thread)

    private func handle(_ line: String) -> String {
        let words = line.split(separator: " ").map(String.init)
        switch (words.first, words.count) {
        case ("ping", 1): return "ok"
        case ("hide", 1): hide(); return "ok"
        case ("dump", 1): return popup.isRunning ? popup.visibleText() : "error no popup is open"
        case ("request-location", 1): requestLocation(); return "ok"
        case ("toggle", 2), ("show", 2):
            guard ModuleLookup.find(words[1]) != nil else { return "error module not found: telmo-\(words[1])" }
            if words[0] == "toggle" { toggle(words[1]) } else { show(words[1]) }
            return "ok"
        default: return "error unknown command: \(line)"
        }
    }

    private func toggle(_ module: String) {
        if popup.isRunning && self.module == module { hide() } else { show(module) }
    }

    private func show(_ module: String) {
        guard let exe = ModuleLookup.find(module) else {
            NSLog("telmo: telmo-\(module) not found")
            return
        }
        if !popup.isRunning {
            let front = NSWorkspace.shared.frontmostApplication
            previousApp = front?.processIdentifier == getpid() ? nil : front
        }
        self.module = module
        popup.run(executable: exe)
        dim.show(below: popup)
        NSApp.activate()
        popup.show()
    }

    private func hide() {
        popup.terminateChild()
        close()
    }

    /// Called once the child is gone (or after hide asked it to go).
    private func close() {
        module = nil
        popup.hide()
        dim.hide()
        previousApp?.activate()
        previousApp = nil
    }

    // MARK: Location

    private func requestLocation() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        let manager = CLLocationManager()
        manager.delegate = self
        locationManager = manager
        manager.requestWhenInUseAuthorization()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined else { return }
        NSApp.setActivationPolicy(.accessory)
        locationManager = nil
    }
}
