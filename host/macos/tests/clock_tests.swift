// Run with tests/run.sh. Covers the decision logic of ClockWatcher: what is due, what is announced once.
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

@main struct ClockTests {
    static func main() {
        let t0: Int64 = 1_790_000_000_000
        typealias T = ClockFile.Timer
        typealias A = ClockFile.Alarm

        // Reads what telmo-clock writes, and tolerates older or partial files.
        let json = """
        {"timers":[{"id":"a1","name":"tea","duration_ms":300000,"ends_at_ms":1790000300000,"paused_remaining_ms":null,"fired":false},
                   {"id":"b2","name":"","duration_ms":60000,"ends_at_ms":null,"paused_remaining_ms":20000,"fired":false}],
         "stopwatch":{"started_at_ms":null,"elapsed_ms":0,"laps":[]},
         "alarms":[{"id":"c3","name":"Wake up","hour":7,"minute":30,"days":[1,2],"enabled":true,"next_ms":1790100000000,"last_fired_ms":null}],
         "recent":[300000]}
        """
        let file = ClockFile.decode(Data(json.utf8))
        check(file?.timers.count == 2 && file?.alarms.count == 1, "decodes timers and alarms")
        check(file?.timers[0].dueMs == t0 + 300_000, "running timer is due at ends_at")
        check(file?.timers[1].dueMs == nil, "paused timer is not scheduled")
        check(file?.alarms[0].dueMs == 1_790_100_000_000, "enabled alarm is due at next_ms")
        check(ClockFile.decode(Data("{}".utf8)) == ClockFile(), "empty object is an empty file")
        check(ClockFile.decode(Data("not json".utf8)) == nil, "garbage is no file")

        // Nothing scheduled: no polling.
        check(!ClockLogic.hasPending(nil), "no file, no polling")
        check(!ClockLogic.hasPending(ClockFile()), "empty file, no polling")
        check(!ClockLogic.hasPending(ClockFile(timers: [T(id: "x", endsAtMs: t0, fired: true)])), "fired timer, no polling")
        check(!ClockLogic.hasPending(ClockFile(timers: [T(id: "x", endsAtMs: nil, pausedRemainingMs: 5)])), "paused timer, no polling")
        check(!ClockLogic.hasPending(ClockFile(alarms: [A(id: "y", enabled: false, nextMs: t0)])), "disabled alarm, no polling")
        check(ClockLogic.hasPending(file), "pending timer starts the poll")

        // Due only once the time has come.
        let tea = T(id: "a1", name: "tea", durationMs: 300_000, endsAtMs: t0 + 300_000)
        let running = ClockFile(timers: [tea])
        check(ClockLogic.due(running, nowMs: t0 + 299_999).isEmpty, "not due a millisecond early")
        let due = ClockLogic.due(running, nowMs: t0 + 300_000)
        check(due.count == 1 && due[0].title == "Timer done" && due[0].body == "tea · 5m", "due on time with its words")
        check(ClockLogic.due(ClockFile(timers: [T(id: "z", durationMs: 1_500_000, endsAtMs: t0)]), nowMs: t0)[0].body == "25m timer", "unnamed timer says its length")

        // Announced once, even if marking it fired fails and the file still says due.
        let handled: Set<String> = [due[0].key]
        check(ClockLogic.due(running, nowMs: t0 + 400_000, handled: handled).isEmpty, "handled ring is not repeated")
        check(!ClockLogic.hasPending(running, handled: handled), "handled ring stops the poll")
        let moved = ClockFile(timers: [T(id: "a1", name: "tea", durationMs: 300_000, endsAtMs: t0 + 900_000)])
        check(ClockLogic.due(moved, nowMs: t0 + 900_000, handled: handled).count == 1, "a restarted timer rings again")

        // Alarms: on time, and slept through.
        let wake = ClockFile(alarms: [A(id: "c3", name: "Wake up", hour: 7, minute: 30, nextMs: t0)])
        let ring = ClockLogic.due(wake, nowMs: t0 + 1_000)
        check(ring.count == 1 && ring[0].title == "Alarm" && ring[0].body == "Wake up · 07:30" && !ring[0].missed, "alarm on time")
        let late = ClockLogic.due(wake, nowMs: t0 + 120_001)
        check(late[0].missed && late[0].title == "Missed alarm", "alarm slept through is missed")
        check(!ClockLogic.due(wake, nowMs: t0 + 120_000)[0].missed, "two minutes late still rings")
        check(ClockLogic.due(ClockFile(alarms: [A(id: "q", hour: 6, minute: 5, nextMs: t0)]), nowMs: t0)[0].body == "06:05", "unnamed alarm shows its time")

        // Several at once come oldest first.
        let both = ClockFile(timers: [T(id: "t", endsAtMs: t0 + 20)], alarms: [A(id: "a", nextMs: t0 + 10)])
        check(ClockLogic.due(both, nowMs: t0 + 30).map(\.id) == ["a", "t"], "oldest first")

        // Lengths read like telmo-clock's.
        check(ClockLogic.shortLength(300_000) == "5m" && ClockLogic.shortLength(5_400_000) == "1h30m", "short lengths")
        check(ClockLogic.shortLength(90_000) == "1m30s" && ClockLogic.shortLength(45_000) == "45s", "short lengths, seconds")

        // Paths.
        check(ClockFile.path(env: [:], home: "/h") == "/h/.local/state/telmo/clock.json", "default path")
        check(ClockFile.path(env: ["XDG_STATE_HOME": "/x"], home: "/h") == "/x/telmo/clock.json", "XDG path")
        check(ClockFile.path(env: ["TELMO_STATE_DIR": "/d"], home: "/h") == "/d/clock.json", "override path")

        print("clock tests passed")
    }
}
