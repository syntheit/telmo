import AppKit
import CoreLocation

final class AppDelegate: NSObject, NSApplicationDelegate, CLLocationManagerDelegate {
    private let popup = PopupPanel()
    private let dim = DimWindows()
    private let island = RebuildIsland()
    private let bluetooth = BluetoothGuard()
    private let equalizer = Equalizer()
    private var hotkeys: Hotkeys?
    private var clipboard: ClipboardWatcher?
    private let clock = ClockWatcher()
    private var ipc: IPCServer?
    private var locationManager: CLLocationManager?
    private var module: String?
    private var signalSources: [DispatchSourceSignal] = []

    func applicationDidFinishLaunching(_ note: Notification) {
        if IPCServer.isRunning() { exit(0) }
        popup.onExit = { [weak self] in self?.close() }
        popup.onEscape = { [weak self] in self?.hide() }
        dim.onClick = { [weak self] in self?.hide() }
        island.isPopupOpen = { [weak self] in self?.module == "system" }
        island.onClick = { [weak self] in self?.show("system") }

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

        bluetooth.start()
        equalizer.start()
        island.start()
        clock.start()
        if ClipboardWatcher.isEnabled {
            clipboard = ClipboardWatcher()
            clipboard?.start()
        }
        hotkeys = Hotkeys { [weak self] module in self?.toggle(module) }
    }

    func applicationWillTerminate(_ note: Notification) {
        popup.terminateChild() // otherwise the telmo-* child outlives the host
        equalizer.stop()
        clock.stop()
    }

    // MARK: Commands (main thread)

    private func handle(_ line: String) -> String {
        let words = line.split(separator: " ").map(String.init)
        switch (words.first, words.count) {
        case ("ping", 1): return "ok"
        case ("hide", 1): hide(); return "ok"
        case ("dump", 1): return popup.isRunning ? popup.visibleText() : "error no popup is open"
        case ("eq", 2) where words[1] == "reload": equalizer.reload(); return "ok"
        case ("location-status", 1): return locationStatus()
        case ("request-location", 1): return requestLocation()
        case ("toggle", 2), ("show", 2):
            if case .failure(let error) = Popups.resolve(words[1]) { return "error \(error.message)" }
            if words[0] == "toggle" { toggle(words[1]) } else { show(words[1]) }
            return "ok"
        default: return bluetooth.handle(words) ?? "error unknown command: \(line)"
        }
    }

    private func toggle(_ module: String) {
        if popup.isRunning && self.module == module { hide() } else { show(module) }
    }

    private func show(_ module: String) {
        guard case .success(let launch) = Popups.resolve(module) else {
            NSLog("telmo: popup '\(module)' not found")
            return
        }
        self.module = module
        popup.run(launch)
        dim.show(below: popup)
        popup.show()
        FileHandle.standardError.write(Data("telmo: popup key=\(popup.isKeyWindow) active=\(NSApp.isActive)\n".utf8))
    }

    private func hide() {
        popup.terminateChild()
        close()
    }

    /// Called once the child is gone (or after hide asked it to go).
    private func close() {
        let wasSystem = module == "system"
        // Lets the island see a finish as seen (popup still counts as open) and learn whether a build is running.
        let rebuilding = wasSystem && island.isRebuilding()
        if wasSystem { island.tick() }
        module = nil
        // Rebuild still running: the popup shrinks into the notch instead of vanishing.
        if rebuilding, island.canAnimate, let target = island.collapsedFrame(), let image = popup.snapshot() {
            let from = popup.frame
            popup.hide()
            dim.hide()
            island.minimize(image: image, from: from, to: target)
            return
        }
        popup.hide()
        dim.hide()
    }

    // MARK: Location

    private func locationStatus() -> String {
        guard CLLocationManager.locationServicesEnabled() else { return "services-off" }
        switch CLLocationManager().authorizationStatus {
        case .notDetermined: return "not-determined"
        case .restricted: return "restricted"
        case .denied: return "denied"
        default: return "authorized"
        }
    }

    /// The popup and dim windows sit above everything, so close them before the system prompt or Settings appears.
    private func requestLocation() -> String {
        let status = locationStatus()
        switch status {
        case "authorized": return status
        case "not-determined":
            hide()
            NSApp.setActivationPolicy(.regular)
            NSApp.activate()
            let manager = CLLocationManager()
            manager.delegate = self
            locationManager = manager
            manager.requestWhenInUseAuthorization()
            return "prompted"
        default:
            hide()
            openLocationSettings()
            return "opened-settings"
        }
    }

    private func openLocationSettings() {
        let urls = [
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_LocationServices",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices",
        ]
        for text in urls {
            if let url = URL(string: text), NSWorkspace.shared.open(url) { return }
        }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        guard manager.authorizationStatus != .notDetermined else { return }
        NSApp.setActivationPolicy(.accessory)
        locationManager = nil
        if locationStatus() == "authorized" { show("net") }
    }
}
