// Run with tests/run.sh. Covers RebuildStatus parsing, the island text and when it shows.
import CoreGraphics
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

func status(_ state: String, pid: Int = 1, built: Int = 0, toBuild: Int = 0, fetched: Int = 0, toFetch: Int = 0, gen: Int? = nil) -> RebuildStatus {
    let g = gen.map(String.init) ?? "null"
    let json = """
    {"state":"\(state)","pid":\(pid),"started":1000,"finished":null,"built":\(built),"to_build":\(toBuild),\
    "fetched":\(fetched),"to_fetch":\(toFetch),"last_line":"x","error":null,"generation":\(g)}
    """
    return RebuildStatus.decode(Data(json.utf8))!
}

@main struct IslandTests {
    static func main() {
        let t0 = Date(timeIntervalSince1970: 1000)

        // Parsing and paths.
        check(RebuildStatus.decode(Data("nope".utf8)) == nil, "garbage rejected")
        check(status("failed").state == .failed, "decodes state")
        check(RebuildStatus.path(env: [:], home: "/h") == "/h/.local/state/telmo/rebuild.json", "default path")
        check(RebuildStatus.path(env: ["XDG_STATE_HOME": "/x"], home: "/h") == "/x/telmo/rebuild.json", "xdg path")
        check(RebuildStatus.path(env: ["TELMO_STATE_DIR": "/d", "XDG_STATE_HOME": "/x"], home: "/h") == "/d/rebuild.json", "override path")

        // Text and fraction.
        let counted = IslandContent.make(status("running", built: 12, toBuild: 40), now: t0)
        check(counted.text == "12/40" && counted.fraction == 0.3, "built/to_build")
        let fetched = IslandContent.make(status("running", fetched: 3, toFetch: 6), now: t0)
        check(fetched.text == "3/6" && fetched.fraction == 0.5, "fetch fallback")
        let elapsed = IslandContent.make(status("running"), now: t0.addingTimeInterval(151))
        check(elapsed.text == "2:31" && elapsed.fraction == nil, "elapsed fallback")
        check(IslandContent.make(status("running", built: 9, toBuild: 4), now: t0).fraction == 1, "fraction clamped")
        check(IslandContent.make(status("ok", gen: 142), now: t0).text == "gen 142", "generation")
        check(IslandContent.make(status("ok"), now: t0).text == "done", "no generation")
        check(IslandContent.make(status("failed"), now: t0).kind == .failed, "failed kind")

        // Running shows, unless the popup is open.
        var tracker = IslandTracker()
        check(tracker.update(status("running"), popupOpen: false, now: t0)?.kind == .running, "running shown at launch")
        check(tracker.update(status("running"), popupOpen: true, now: t0) == nil, "hidden while popup open")
        check(tracker.update(nil, popupOpen: false, now: t0) == nil, "no file")

        // Finish after running: ok shows 10 s, then goes and stays gone.
        check(tracker.update(status("ok", gen: 5), popupOpen: false, now: t0)?.kind == .ok, "ok shown")
        check(tracker.update(status("ok", gen: 5), popupOpen: false, now: t0.addingTimeInterval(9)) != nil, "ok still shown at 9s")
        check(tracker.update(status("ok", gen: 5), popupOpen: false, now: t0.addingTimeInterval(10)) == nil, "ok retracts at 10s")
        check(tracker.update(status("ok", gen: 5), popupOpen: false, now: t0.addingTimeInterval(11)) == nil, "ok stays gone")

        // A finish seen while the popup is open is seen.
        tracker = IslandTracker()
        _ = tracker.update(status("running", pid: 2), popupOpen: true, now: t0)
        check(tracker.update(status("ok", pid: 2), popupOpen: true, now: t0) == nil, "finish under popup hidden")
        check(tracker.update(status("ok", pid: 2), popupOpen: false, now: t0) == nil, "not shown after popup closes")
        check(tracker.update(status("running", pid: 3), popupOpen: false, now: t0) != nil, "next run shows again")

        // Old results at launch are ignored; a later failure stays until acknowledged.
        tracker = IslandTracker()
        check(tracker.update(status("failed", pid: 4), popupOpen: false, now: t0) == nil, "old result at launch ignored")
        _ = tracker.update(status("running", pid: 5), popupOpen: false, now: t0)
        let failed = status("failed", pid: 5)
        check(tracker.update(failed, popupOpen: false, now: t0)?.kind == .failed, "failure shown")
        check(tracker.update(failed, popupOpen: false, now: t0.addingTimeInterval(600)) != nil, "failure persists")
        tracker.acknowledge(failed)
        check(tracker.update(failed, popupOpen: false, now: t0.addingTimeInterval(601)) == nil, "acknowledged failure hidden")

        // A second success gets its own 10 s.
        tracker = IslandTracker()
        _ = tracker.update(status("running", pid: 6), popupOpen: false, now: t0)
        _ = tracker.update(status("ok", pid: 6), popupOpen: false, now: t0)
        check(tracker.update(status("ok", pid: 6), popupOpen: false, now: t0.addingTimeInterval(10)) == nil, "first ok retracts")
        _ = tracker.update(status("running", pid: 7), popupOpen: false, now: t0.addingTimeInterval(60))
        check(tracker.update(status("ok", pid: 7), popupOpen: false, now: t0.addingTimeInterval(70))?.kind == .ok, "second ok shown")

        // A dead runner that still says running is a failure.
        check(status("running").checked(alive: { _ in false }).state == .failed, "dead runner failed")
        check(status("running").checked(alive: { _ in true }).state == .running, "live runner running")
        check(status("ok").checked(alive: { _ in false }).state == .ok, "finished untouched")
        check(RebuildStatus.pidAlive(Int(getpid())) && !RebuildStatus.pidAlive(0), "pid check")

        // Geometry.
        let screen = CGRect(x: 0, y: 0, width: 1512, height: 982)
        let notch = IslandGeometry.notchRect(screen: screen, topInset: 38, leftArea: CGRect(x: 0, y: 944, width: 660, height: 38),
                                             rightArea: CGRect(x: 852, y: 944, width: 660, height: 38))
        check(notch == CGRect(x: 660, y: 944, width: 192, height: 38), "notch rect")
        check(IslandGeometry.notchRect(screen: screen, topInset: 0, leftArea: nil, rightArea: nil) == nil, "no notch")
        let pill = IslandGeometry.pillFrame(screen: screen, notch: notch)
        check(pill.midX == screen.midX && pill.maxY == screen.maxY && pill.width > notch!.width, "pill flush top, centered, wider")
        let cap = IslandGeometry.pillFrame(screen: screen, notch: nil)
        check(cap.midX == screen.midX && cap.maxY < screen.maxY, "capsule below top")

        print("island tests passed")
    }
}
