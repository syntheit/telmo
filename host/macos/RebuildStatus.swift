import CoreGraphics
import Foundation

/// The parts of `rebuild.json` (written by telmo-system's detached rebuild) that the island shows.
struct RebuildStatus: Decodable, Equatable {
    enum State: String, Decodable { case running, ok, failed }
    enum Phase: String, Decodable { case evaluating, downloading, building, activating }

    var state: State
    let pid: Int
    let started: Int
    let built: Int
    let toBuild: Int
    let fetched: Int
    let toFetch: Int
    let generation: Int?
    /// Files from older versions have none, which counts as evaluating.
    let phase: Phase

    enum CodingKeys: String, CodingKey {
        case state, pid, started, built, generation, fetched, phase
        case toBuild = "to_build"
        case toFetch = "to_fetch"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        state = try c.decode(State.self, forKey: .state)
        pid = try c.decode(Int.self, forKey: .pid)
        started = try c.decode(Int.self, forKey: .started)
        built = try c.decode(Int.self, forKey: .built)
        toBuild = try c.decode(Int.self, forKey: .toBuild)
        fetched = try c.decode(Int.self, forKey: .fetched)
        toFetch = try c.decode(Int.self, forKey: .toFetch)
        generation = try c.decodeIfPresent(Int.self, forKey: .generation)
        phase = (try? c.decodeIfPresent(Phase.self, forKey: .phase)) ?? .evaluating
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

/// What the island draws: the lit bars, and which of them moves.
struct IslandContent: Equatable {
    /// How many bars the right wing has.
    static let bars = 10

    enum Look: Equatable {
        case evaluating, downloading, building, activating, ok, failed

        /// The colour of the bars, as 0xRRGGBB.
        var rgb: UInt32 {
            switch self {
            case .evaluating: 0x9aa0b8
            case .downloading: 0x7ebae4
            // Green is kept for done, so the finish stands out from almost done.
            case .building, .activating: 0x5b86d6
            case .ok: 0x9ece6a
            case .failed: 0xf7768e
            }
        }
    }

    let look: Look
    /// Bars at full brightness, 0...bars.
    let lit: Int
    /// The bar that breathes: the first unlit one while running.
    let head: Int?
    /// A highlight sweeps over dim bars while no totals are known.
    let sweeping: Bool

    static func make(_ status: RebuildStatus) -> IslandContent {
        let count = fraction(status).map { min(bars, Int($0 * Double(bars))) }
        switch status.state {
        case .running:
            let look: Look = switch status.phase {
            case .evaluating: .evaluating
            case .downloading: .downloading
            case .building: .building
            case .activating: .activating
            }
            // Activation has no count: everything is lit but the last bar.
            let lit = status.phase == .activating ? bars - 1 : count ?? 0
            return IslandContent(look: look, lit: lit, head: lit < bars ? lit : nil,
                                 sweeping: look == .evaluating && count == nil)
        case .ok:
            return IslandContent(look: .ok, lit: bars, head: nil, sweeping: false)
        case .failed:
            // A failed switch keeps the activation picture, just red.
            let lit = status.phase == .activating ? bars - 1 : max(1, count ?? 0)
            return IslandContent(look: .failed, lit: lit, head: nil, sweeping: false)
        }
    }

    /// fetched/to_fetch while downloading, otherwise built/to_build; whichever is known as a fallback,
    /// nil while no totals are announced.
    static func fraction(_ s: RebuildStatus) -> Double? {
        func ratio(_ done: Int, _ total: Int) -> Double? { total > 0 ? min(1, Double(done) / Double(total)) : nil }
        let builds = ratio(s.built, s.toBuild), fetches = ratio(s.fetched, s.toFetch)
        return s.phase == .downloading ? fetches ?? builds : builds ?? fetches
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
            return popupOpen ? nil : IslandContent.make(status)
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
        return IslandContent.make(status)
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
            // Exactly as tall as the notch, so nothing hangs below it over the menu bar.
            let h = notch.height
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
