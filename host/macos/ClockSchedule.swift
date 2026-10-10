import Foundation

/// The parts of `clock.json` (written only by `telmo-clock`) that decide when to ring.
/// Telmo.app never writes the file: after announcing it runs `telmo-clock fired <id>`.
struct ClockFile: Decodable, Equatable {
    struct Timer: Decodable, Equatable {
        var id: String
        var name: String
        var durationMs: Int64
        var endsAtMs: Int64?
        var pausedRemainingMs: Int64?
        var fired: Bool

        enum CodingKeys: String, CodingKey {
            case id, name, fired
            case durationMs = "duration_ms"
            case endsAtMs = "ends_at_ms"
            case pausedRemainingMs = "paused_remaining_ms"
        }

        init(id: String, name: String = "", durationMs: Int64 = 0, endsAtMs: Int64? = nil, pausedRemainingMs: Int64? = nil, fired: Bool = false) {
            self.id = id; self.name = name; self.durationMs = durationMs
            self.endsAtMs = endsAtMs; self.pausedRemainingMs = pausedRemainingMs; self.fired = fired
        }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
            durationMs = try c.decodeIfPresent(Int64.self, forKey: .durationMs) ?? 0
            endsAtMs = try c.decodeIfPresent(Int64.self, forKey: .endsAtMs)
            pausedRemainingMs = try c.decodeIfPresent(Int64.self, forKey: .pausedRemainingMs)
            fired = try c.decodeIfPresent(Bool.self, forKey: .fired) ?? false
        }

        /// When it must ring; nil if paused or already announced.
        var dueMs: Int64? { fired || pausedRemainingMs != nil ? nil : endsAtMs }
    }

    struct Alarm: Decodable, Equatable {
        var id: String
        var name: String
        var hour: Int
        var minute: Int
        var enabled: Bool
        var nextMs: Int64?

        enum CodingKeys: String, CodingKey {
            case id, name, hour, minute, enabled
            case nextMs = "next_ms"
        }

        init(id: String, name: String = "", hour: Int = 7, minute: Int = 0, enabled: Bool = true, nextMs: Int64? = nil) {
            self.id = id; self.name = name; self.hour = hour; self.minute = minute
            self.enabled = enabled; self.nextMs = nextMs
        }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
            hour = try c.decodeIfPresent(Int.self, forKey: .hour) ?? 7
            minute = try c.decodeIfPresent(Int.self, forKey: .minute) ?? 0
            enabled = try c.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
            nextMs = try c.decodeIfPresent(Int64.self, forKey: .nextMs)
        }

        var dueMs: Int64? { enabled ? nextMs : nil }
    }

    var timers: [Timer] = []
    var alarms: [Alarm] = []

    enum CodingKeys: String, CodingKey { case timers, alarms }

    init(timers: [Timer] = [], alarms: [Alarm] = []) { self.timers = timers; self.alarms = alarms }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        timers = try c.decodeIfPresent([Timer].self, forKey: .timers) ?? []
        alarms = try c.decodeIfPresent([Alarm].self, forKey: .alarms) ?? []
    }

    static func decode(_ data: Data) -> ClockFile? { try? JSONDecoder().decode(ClockFile.self, from: data) }

    /// The state directory: `$TELMO_STATE_DIR` (demos), else `$XDG_STATE_HOME/telmo`, else `~/.local/state/telmo`.
    static func directory(env: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory()) -> String {
        func set(_ key: String) -> String? { env[key].flatMap { $0.isEmpty ? nil : $0 } }
        return set("TELMO_STATE_DIR") ?? (set("XDG_STATE_HOME") ?? home + "/.local/state") + "/telmo"
    }

    static func path(env: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory()) -> String {
        directory(env: env, home: home) + "/clock.json"
    }
}

/// One thing to announce.
struct ClockDue: Equatable {
    enum Kind: Equatable { case timer, alarm }

    let id: String
    let kind: Kind
    let name: String
    /// A timer's length ("5m") or an alarm's time ("07:30").
    let detail: String
    let dueMs: Int64
    /// An alarm slept through: announced quietly.
    let missed: Bool

    /// Identifies this ring, so it is announced once even if marking it fired fails.
    var key: String { "\(id)@\(dueMs)" }

    var title: String {
        switch kind {
        case .timer: "Timer done"
        case .alarm: missed ? "Missed alarm" : "Alarm"
        }
    }

    var body: String {
        let fallback = kind == .timer ? "\(detail) timer" : detail
        return name.isEmpty ? fallback : "\(name) · \(detail)"
    }
}

enum ClockLogic {
    /// An alarm this late was slept through.
    static let missedAfterMs: Int64 = 120_000

    /// Everything scheduled that has not been announced yet.
    static func pending(_ file: ClockFile, handled: Set<String> = []) -> [(id: String, dueMs: Int64)] {
        let timers = file.timers.compactMap { t in t.dueMs.map { (t.id, $0) } }
        let alarms = file.alarms.compactMap { a in a.dueMs.map { (a.id, $0) } }
        return (timers + alarms).filter { !handled.contains("\($0.0)@\($0.1)") }.map { (id: $0.0, dueMs: $0.1) }
    }

    /// The one-second poll runs only while this is true.
    static func hasPending(_ file: ClockFile?, handled: Set<String> = []) -> Bool {
        guard let file else { return false }
        return !pending(file, handled: handled).isEmpty
    }

    /// What to announce at `nowMs`, oldest first.
    static func due(_ file: ClockFile, nowMs: Int64, handled: Set<String> = []) -> [ClockDue] {
        var out: [ClockDue] = []
        for t in file.timers {
            guard let due = t.dueMs, due <= nowMs, !handled.contains("\(t.id)@\(due)") else { continue }
            out.append(ClockDue(id: t.id, kind: .timer, name: t.name, detail: shortLength(t.durationMs), dueMs: due, missed: false))
        }
        for a in file.alarms {
            guard let due = a.dueMs, due <= nowMs, !handled.contains("\(a.id)@\(due)") else { continue }
            let time = String(format: "%02d:%02d", a.hour, a.minute)
            out.append(ClockDue(id: a.id, kind: .alarm, name: a.name, detail: time, dueMs: due, missed: nowMs - due > missedAfterMs))
        }
        return out.sorted { $0.dueMs < $1.dueMs }
    }

    /// `5m`, `1h30m`, `1m30s`, `45s`: the same words `telmo-clock` uses.
    static func shortLength(_ ms: Int64) -> String {
        let secs = (max(0, ms) + 500) / 1000
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60)
        var out = ""
        if h > 0 { out += "\(h)h" }
        if m > 0 { out += "\(m)m" }
        if s > 0 || out.isEmpty { out += "\(s)s" }
        return out
    }
}
