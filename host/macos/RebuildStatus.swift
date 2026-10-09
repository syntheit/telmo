import CoreGraphics
import Foundation

/// The parts of `rebuild.json` (written by telmo-system's detached rebuild) that the island shows.
struct RebuildStatus: Decodable, Equatable {
    enum State: String, Decodable { case running, ok, failed }

    var state: State
    let pid: Int
    let started: Int
    let built: Int
    let toBuild: Int
    let fetched: Int
    let toFetch: Int
    let generation: Int?

    enum CodingKeys: String, CodingKey {
        case state, pid, started, built, generation, fetched
        case toBuild = "to_build"
        case toFetch = "to_fetch"
    }

    /// Where `rebuild.json` lives: `$TELMO_STATE_DIR` (for demos), else `$XDG_STATE_HOME/telmo`, else `~/.local/state/telmo`.
    static func path(env: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory()) -> String {
        func set(_ key: String) -> String? { env[key].flatMap { $0.isEmpty ? nil : $0 } }
        let dir = set("TELMO_STATE_DIR") ?? (set("XDG_STATE_HOME") ?? home + "/.local/state") + "/telmo"
        return dir + "/rebuild.json"
    }

    static func decode(_ data: Data) -> RebuildStatus? { try? JSONDecoder().decode(RebuildStatus.self, from: data) }

    /// A run that says "running" but whose process is gone counts as failed, like telmo-system reads it.
    func checked(alive: (Int) -> Bool = pidAlive) -> RebuildStatus {
        guard state == .running, !alive(pid) else { return self }
        var stopped = self
        stopped.state = .failed
        return stopped
    }

    /// The runner is root, so a live one answers EPERM.
    static func pidAlive(_ pid: Int) -> Bool {
        guard pid > 0, let pid = Int32(exactly: pid) else { return false }
        return kill(pid, 0) == 0 || errno == EPERM
    }

    /// Identifies one rebuild run.
    var key: String { "\(pid)-\(started)" }
}

/// What the island draws.
struct IslandContent: Equatable {
    enum Kind: Equatable { case running, ok, failed }

    let kind: Kind
    let text: String
    /// 0...1, or nil while the totals are unknown.
    let fraction: Double?

    static func make(_ status: RebuildStatus, now: Date) -> IslandContent {
        switch status.state {
        case .running:
            return IslandContent(kind: .running, text: progressText(status, now: now), fraction: fraction(status))
        case .ok:
            return IslandContent(kind: .ok, text: status.generation.map { "gen \($0)" } ?? "done", fraction: 1)
        case .failed:
            return IslandContent(kind: .failed, text: "failed", fraction: nil)
        }
    }

    /// built/to_build, else fetched/to_fetch, else the elapsed time.
    static func progressText(_ s: RebuildStatus, now: Date) -> String {
        if s.toBuild > 0 { return "\(min(s.built, s.toBuild))/\(s.toBuild)" }
        if s.toFetch > 0 { return "\(min(s.fetched, s.toFetch))/\(s.toFetch)" }
        let secs = max(0, Int(now.timeIntervalSince1970) - s.started)
        return String(format: "%d:%02d", secs / 60, secs % 60)
    }

    static func fraction(_ s: RebuildStatus) -> Double? {
        if s.toBuild > 0 { return min(1, Double(s.built) / Double(s.toBuild)) }
        if s.toFetch > 0 { return min(1, Double(s.fetched) / Double(s.toFetch)) }
        return nil
    }
}

/// Decides when the island is visible. Fed the current status once a second.
struct IslandTracker {
    /// How long the finished state stays before the island retracts.
    static let okSeconds: TimeInterval = 10

    private var first = true
    private var handled: String?
    /// The run whose success is showing, and since when.
    private var ok: (key: String, since: Date)?

    /// nil hides the island.
    mutating func update(_ status: RebuildStatus?, popupOpen: Bool, now: Date) -> IslandContent? {
        defer { first = false }
        guard let status else { return nil }
        if status.state == .running {
            return popupOpen ? nil : IslandContent.make(status, now: now)
        }
        // Results from before the app started, or that the popup showed, are old news.
        if first || popupOpen { handled = status.key }
        if handled == status.key { return nil }
        if status.state == .ok {
            if ok?.key != status.key { ok = (status.key, now) }
            if let ok, now.timeIntervalSince(ok.since) >= Self.okSeconds {
                handled = status.key
                return nil
            }
        }
        return IslandContent.make(status, now: now)
    }

    /// The user clicked the island: the current result is seen.
    mutating func acknowledge(_ status: RebuildStatus?) {
        if let status, status.state != .running { handled = status.key }
    }
}

/// Geometry of the island in screen points (bottom-left origin, like AppKit).
enum IslandGeometry {
    static let height: CGFloat = 34
    static let wing: CGFloat = 78
    static let capsuleWidth: CGFloat = 170

    /// Full width and height of the pill. `notch` is the notch rect, nil on screens without one.
    static func pillFrame(screen: CGRect, notch: CGRect?) -> CGRect {
        if let notch {
            let h = max(height, notch.height)
            let width = notch.width + 2 * wing
            return CGRect(x: notch.midX - width / 2, y: screen.maxY - h, width: width, height: h)
        }
        return CGRect(x: screen.midX - capsuleWidth / 2, y: screen.maxY - height - 6, width: capsuleWidth, height: height)
    }

    /// The notch rect from the safe-area insets and the free areas either side of it.
    static func notchRect(screen: CGRect, topInset: CGFloat, leftArea: CGRect?, rightArea: CGRect?) -> CGRect? {
        guard topInset > 0, let leftArea, let rightArea else { return nil }
        return CGRect(x: leftArea.maxX, y: screen.maxY - topInset, width: rightArea.minX - leftArea.maxX, height: topInset)
    }
}
