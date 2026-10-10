import AppKit
import UserNotifications

/// Announces due timers and alarms from `clock.json` while the popup is closed.
///
/// Costs nothing while nothing is scheduled: a directory watch notices when `telmo-clock` rewrites the file
/// (it renames a temp file into place), and the one-second poll runs only while something is pending.
/// Telmo.app never writes the file; `telmo-clock fired <id>` is the single writer.
final class ClockWatcher: NSObject, UNUserNotificationCenterDelegate {
    private var file: ClockFile?
    private var handled = Set<String>()
    private var timer: Timer?
    private var directorySource: DispatchSourceFileSystemObject?
    private let queue = DispatchQueue(label: "telmo.clock.fired")

    func start() {
        let dir = ClockFile.directory()
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let fd = open(dir, O_EVTONLY)
        if fd >= 0 {
            let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd, eventMask: [.write, .rename, .extend], queue: .main)
            source.setEventHandler { [weak self] in self?.reload() }
            source.setCancelHandler { close(fd) }
            source.resume()
            directorySource = source
        }
        // A sleeping Mac rang nothing; look again the moment it wakes.
        NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
            self?.reload()
        }
        reload()
    }

    func stop() {
        timer?.invalidate()
        directorySource?.cancel()
    }

    /// Reads the file and starts or stops the poll.
    private func reload() {
        file = FileManager.default.contents(atPath: ClockFile.path()).flatMap(ClockFile.decode)
        // Forget rings that are no longer in the file.
        let live = Set(file.map { ClockLogic.pending($0).map { "\($0.id)@\($0.dueMs)" } } ?? [])
        handled.formIntersection(live)
        if ClockLogic.hasPending(file, handled: handled) {
            if timer == nil {
                let t = Timer(timeInterval: 1, repeats: true) { [weak self] _ in self?.tick() }
                RunLoop.main.add(t, forMode: .common)
                timer = t
            }
            tick()
        } else {
            timer?.invalidate()
            timer = nil
        }
    }

    private func tick() {
        guard let file else { return }
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        for due in ClockLogic.due(file, nowMs: now, handled: handled) {
            handled.insert(due.key)
            announce(due)
            markFired(due.id)
        }
        if !ClockLogic.hasPending(file, handled: handled) {
            timer?.invalidate()
            timer = nil
        }
    }

    private func announce(_ due: ClockDue) {
        if !due.missed { NSSound(named: "Glass")?.play() }
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let post = {
            let content = UNMutableNotificationContent()
            content.title = due.title
            content.body = due.body
            center.add(UNNotificationRequest(identifier: "telmo.clock.\(due.key)", content: content, trigger: nil))
        }
        center.getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .authorized, .provisional: post()
            case .notDetermined:
                // The first ring asks; the sound has already played if the answer is no.
                center.requestAuthorization(options: [.alert]) { granted, _ in if granted { post() } }
            default: break // denied: the sound is all there is
            }
        }
    }

    /// Hands the ring back to `telmo-clock`, the file's only writer.
    private func markFired(_ id: String) {
        guard let exe = ModuleLookup.find("clock") else {
            NSLog("telmo: telmo-clock not found; cannot mark \(id) fired")
            return
        }
        queue.async {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: exe)
            p.arguments = ["fired", id]
            p.standardOutput = FileHandle.nullDevice
            p.standardError = FileHandle.nullDevice
            try? p.run()
            p.waitUntilExit()
        }
    }

    /// Show banners even while a Telmo popup has the focus.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }
}
