// Run with tests/run.sh. Covers RebuildStatus parsing, what the island draws and when it shows.
import CoreGraphics
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

func status(_ state: String, pid: Int = 1, built: Int = 0, toBuild: Int = 0, fetched: Int = 0, toFetch: Int = 0, gen: Int? = nil, phase: String? = nil) -> RebuildStatus {
    let g = gen.map(String.init) ?? "null"
    let p = phase.map { "\"phase\":\"\($0)\"," } ?? ""
    let json = """
    {\(p)"state":"\(state)","pid":\(pid),"started":1000,"finished":null,"built":\(built),"to_build":\(toBuild),\
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

        // Phase: absent means evaluating, unknown values too.
        check(status("running").phase == .evaluating, "no phase decodes as evaluating")
        check(status("running", phase: "nonsense").phase == .evaluating, "unknown phase decodes as evaluating")
        for (name, phase) in [("evaluating", RebuildStatus.Phase.evaluating), ("downloading", .downloading),
                              ("building", .building), ("activating", .activating)] {
            check(status("running", phase: name).phase == phase, "decodes phase \(name)")
        }

        // Lit bars: floor(fraction * 10); downloads count fetches, everything else builds.
        func content(_ s: RebuildStatus) -> IslandContent { IslandContent.make(s) }
        check(content(status("running", built: 12, toBuild: 40, phase: "building")).lit == 3, "12/40 lights 3")
        check(content(status("running", built: 39, toBuild: 40, phase: "building")).lit == 9, "39/40 lights 9")
        check(content(status("running", built: 9, toBuild: 4, phase: "building")).lit == 10, "overshoot clamps to 10")
        check(content(status("running", fetched: 3, toFetch: 6, phase: "downloading")).lit == 5, "fetch fallback")
        check(content(status("running", built: 1, toBuild: 4, fetched: 6, toFetch: 6, phase: "building")).lit == 2, "builds win over fetches")
        check(content(status("running", built: 0, toBuild: 4, fetched: 3, toFetch: 6, phase: "downloading")).lit == 5, "downloading counts fetches")
        check(content(status("running", phase: "downloading")).lit == 0, "no totals lights nothing")
        check(content(status("failed", built: 4, toBuild: 4, phase: "activating")).lit == 9, "failed switch keeps 9")

        // The breathing head is the first unlit bar, and goes when all are lit.
        check(content(status("running", built: 12, toBuild: 40, phase: "building")).head == 3, "head after the lit bars")
        check(content(status("running", phase: "downloading")).head == 0, "head on the first bar")
        check(content(status("running", built: 4, toBuild: 4, phase: "building")).head == nil, "no head when full")

        // Evaluating sweeps until a total is known.
        check(content(status("running")).sweeping && content(status("running")).look == .evaluating, "evaluating sweeps")
        check(!content(status("running", toBuild: 5)).sweeping, "totals stop the sweep")
        check(!content(status("running", phase: "building")).sweeping, "building does not sweep")

        // Colours per phase.
        check(content(status("running")).look.rgb == 0x9aa0b8, "evaluating grey")
        check(content(status("running", phase: "downloading")).look.rgb == 0x7ebae4, "downloading light blue")
        check(content(status("running", phase: "building")).look.rgb == 0x5b86d6, "building deep blue")
        check(content(status("running", phase: "activating")).look.rgb == 0x5b86d6, "activating stays building blue")
        check(content(status("ok")).look.rgb == 0x9ece6a, "success green")
        check(content(status("failed")).look.rgb == 0xf7768e, "failed red")

        // Activating: all lit but the breathing last bar, whatever the counts say.
        let activating = content(status("running", built: 4, toBuild: 4, phase: "activating"))
        check(activating.lit == 9 && activating.head == 9 && !activating.sweeping, "activating lights 9, head on the last")

        // Success: all ten lit, nothing breathes.
        let ok = content(status("ok", gen: 142))
        check(ok.look == .ok && ok.lit == 10 && ok.head == nil && !ok.sweeping, "success lights all")

        // Failure keeps the lit bars, at least one.
        let lost = content(status("failed", built: 12, toBuild: 40, phase: "building"))
        check(lost.look == .failed && lost.lit == 3 && lost.head == nil, "failure keeps 3 lit")
        check(content(status("failed", phase: "building")).lit == 1, "failure with none lit shows 1")
        check(content(status("failed", built: 1, toBuild: 40, phase: "building")).lit == 1, "failure under 10% shows 1")

        // Running shows, unless the popup is open.
        var tracker = IslandTracker()
        check(tracker.update(status("running"), popupOpen: false, now: t0)?.look == .evaluating, "running shown at launch")
        check(tracker.update(status("running"), popupOpen: true, now: t0) == nil, "hidden while popup open")
        check(tracker.update(nil, popupOpen: false, now: t0) == nil, "no file")

        // Finish after running: ok shows 10 s, then goes and stays gone.
        check(tracker.update(status("ok", gen: 5), popupOpen: false, now: t0)?.look == .ok, "ok shown")
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
        check(tracker.update(failed, popupOpen: false, now: t0)?.look == .failed, "failure shown")
        check(tracker.update(failed, popupOpen: false, now: t0.addingTimeInterval(600)) != nil, "failure persists")
        tracker.acknowledge(failed)
        check(tracker.update(failed, popupOpen: false, now: t0.addingTimeInterval(601)) == nil, "acknowledged failure hidden")

        // A second success gets its own 10 s.
        tracker = IslandTracker()
        _ = tracker.update(status("running", pid: 6), popupOpen: false, now: t0)
        _ = tracker.update(status("ok", pid: 6), popupOpen: false, now: t0)
        check(tracker.update(status("ok", pid: 6), popupOpen: false, now: t0.addingTimeInterval(10)) == nil, "first ok retracts")
        _ = tracker.update(status("running", pid: 7), popupOpen: false, now: t0.addingTimeInterval(60))
        check(tracker.update(status("ok", pid: 7), popupOpen: false, now: t0.addingTimeInterval(70))?.look == .ok, "second ok shown")

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
