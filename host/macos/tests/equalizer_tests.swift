// Run with tests/run.sh. Covers EqualizerDSP.swift: eq.json, key, biquads, preamp, limiter and the settings hand-over. No audio devices.
import Foundation

func check(_ ok: Bool, _ what: String) {
    if !ok { print("FAIL: \(what)"); exit(1) }
}

func close(_ a: Double, _ b: Double, _ tolerance: Double) -> Bool { abs(a - b) <= tolerance }

func spec(_ type: String, _ freq: Double, _ gain: Double, _ q: Double) -> EQFilterSpec {
    EQFilterSpec(type: type, freq: freq, gain: gain, q: q)
}

func biquad(_ type: String, _ freq: Double, _ gain: Double, _ q: Double) -> Biquad {
    guard let filter = Biquad(spec(type, freq, gain, q), rate: 48_000) else { fatalError("no filter for \(type) \(freq)") }
    return filter
}

/// Gain of one biquad at a frequency, in dB.
func response(_ q: Biquad, at hz: Double, rate: Double = 48_000) -> Double {
    let w = 2 * Double.pi * hz / rate
    let (c1, s1, c2, s2) = (cos(w), -sin(w), cos(2 * w), -sin(2 * w))
    let (nr, ni) = (q.b0 + q.b1 * c1 + q.b2 * c2, q.b1 * s1 + q.b2 * s2)
    let (dr, di) = (1 + q.a1 * c1 + q.a2 * c2, q.a1 * s1 + q.a2 * s2)
    return 10 * log10((nr * nr + ni * ni) / (dr * dr + di * di))
}

/// Runs planar channels through a processor in one go.
func run(_ processor: EQProcessor, _ channels: [[Float]]) -> [[Float]] {
    let frames = channels[0].count
    let ins = channels.map { channel -> UnsafeMutablePointer<Float> in
        let p = UnsafeMutablePointer<Float>.allocate(capacity: frames)
        p.initialize(from: channel, count: frames)
        return p
    }
    let outs = channels.map { _ in UnsafeMutablePointer<Float>.allocate(capacity: frames) }
    defer { (ins + outs).forEach { $0.deallocate() } }
    for c in channels.indices {
        processor.inputs[c] = EQPlane(data: ins[c], stride: 1)
        processor.outputs[c] = EQPlane(data: outs[c], stride: 1)
    }
    processor.process(frames: frames, channels: channels.count)
    return outs.map { Array(UnsafeBufferPointer(start: $0, count: frames)) }
}

func sine(_ hz: Double, amplitude: Double, frames: Int, rate: Double = 48_000) -> [Float] {
    (0..<frames).map { Float(amplitude * sin(2 * Double.pi * hz * Double($0) / rate)) }
}

func rms(_ samples: ArraySlice<Float>) -> Double {
    (samples.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(samples.count)).squareRoot()
}

@main struct EqualizerTests {
    static func main() {
        coefficients()
        configuration()
        processing()
        handOver()
        print("ok")
    }

    static func coefficients() {
        // Reference values from the RBJ cookbook at 48 kHz, computed independently.
        let peak = biquad("peak", 1000, 6, 1.0)
        check(close(peak.b0, 1.043953086990335, 1e-12) && close(peak.b1, -1.895320723936596, 1e-12)
            && close(peak.b2, 0.867722284759857, 1e-12) && close(peak.a1, -1.895320723936596, 1e-12)
            && close(peak.a2, 0.911675371750192, 1e-12), "peak 1 kHz +6 dB coefficients")
        let low = biquad("lowshelf", 100, 3, 0.7)
        check(close(low.b0, 1.001617651394865, 1e-12) && close(low.b1, -1.982820780559568, 1e-12)
            && close(low.b2, 0.981405042611525, 1e-12) && close(low.a1, -1.982850265396235, 1e-12)
            && close(low.a2, 0.982993209169723, 1e-12), "low shelf 100 Hz +3 dB coefficients")
        let high = biquad("highshelf", 8000, -2, 0.7)
        check(close(high.b0, 0.860122094100095, 1e-12) && close(high.b1, -0.479102894464121, 1e-12)
            && close(high.b2, 0.191470226929099, 1e-12) && close(high.a1, -0.677270838965604, 1e-12)
            && close(high.a2, 0.249760265530677, 1e-12), "high shelf 8 kHz -2 dB coefficients")
        let pass = biquad("highpass", 40, 0, 0.7071067811865476)
        check(close(pass.b0, 0.996304442969349, 1e-12) && close(pass.b1, -1.992608885938698, 1e-12)
            && close(pass.a1, -1.992595228750302, 1e-12) && close(pass.a2, 0.992622543127095, 1e-12), "high-pass 40 Hz coefficients")

        // The gain at the centre of a peak is the gain asked for.
        for gain in [-6.0, 3.0, 8.0] {
            check(close(response(biquad("peak", 160, gain, 0.7), at: 160), gain, 1e-9), "peak centre gain \(gain)")
        }
        check(close(response(biquad("peak", 31.5, 8, 1.4), at: 31.5), 8, 1e-9), "peak centre at 31.5 Hz")
        check(close(response(peak, at: 20_000), 0, 0.3), "peak is flat far away")
        check(close(response(low, at: 10), 3, 0.05) && close(response(low, at: 15_000), 0, 0.05), "low shelf reaches its gain below and none above")
        check(close(response(high, at: 22_000), -2, 0.5) && close(response(high, at: 100), 0, 0.05), "high shelf")
        check(close(response(pass, at: 40), -3.0103, 1e-3) && response(pass, at: 5) < -30 && close(response(pass, at: 2000), 0, 0.01),
              "high-pass is -3 dB at its corner")

        check(Biquad(spec("bandpass", 1000, 0, 1), rate: 48_000) == nil, "unknown type")
        check(Biquad(spec("peak", 24_000, 3, 1), rate: 48_000) == nil, "frequency at Nyquist")
        check(Biquad(spec("peak", 1000, 3, 0), rate: 48_000) == nil, "zero Q")
        check(Biquad(spec("peak", 20_000, 3, 1), rate: 16_000) == nil, "a rate too low for the filter drops it")
    }

    static func configuration() {
        let json = """
        {"enabled": true, "devices": {
          "BuiltInSpeakerDevice#ispk": {"name": "MacBook Air Speakers", "preset": "Speakers +", "bass": 0, "preamp": -3.0,
            "filters": [{"type": "peak", "freq": 160.0, "gain": 3.0, "q": 0.7}]},
          "AA-BB:output": {"name": "EarFun Air Pro 4", "preset": "Flat", "bass": 0, "preamp": 0.0, "filters": []}}}
        """
        guard let config = try? JSONDecoder().decode(EQConfig.self, from: Data(json.utf8)) else { return check(false, "decodes") }
        check(config.enabled && config.devices.count == 2, "enabled, two devices")
        let speakers = config.devices["BuiltInSpeakerDevice#ispk"]
        check(speakers?.preset == "Speakers +" && speakers?.preamp == -3 && speakers?.filters == [spec("peak", 160, 3, 0.7)], "speakers entry")
        check(speakers?.isFlat == false && config.devices["AA-BB:output"]?.isFlat == true, "flat means no filters and no preamp")
        check(EQEntry(preset: "Quiet", preamp: -2, filters: []).isFlat == false, "a preamp alone is not flat")

        let off = try? JSONDecoder().decode(EQConfig.self, from: Data(#"{"enabled": false, "devices": {}}"#.utf8))
        check(off?.enabled == false, "disabled")
        let empty = try? JSONDecoder().decode(EQConfig.self, from: Data("{}".utf8))
        check(empty == EQConfig() && empty?.enabled == true, "missing fields mean on, no devices")
        check((try? JSONDecoder().decode(EQConfig.self, from: Data("nope".utf8))) == nil, "garbage does not decode")

        let path = NSTemporaryDirectory() + "telmo-eq-test-\(UUID().uuidString).json"
        defer { try? FileManager.default.removeItem(atPath: path) }
        check(EQConfig.load(from: path) == nil, "missing file")
        FileManager.default.createFile(atPath: path, contents: Data(json.utf8))
        check(EQConfig.load(from: path) == config, "file loads")
        FileManager.default.createFile(atPath: path, contents: Data("{".utf8))
        check(EQConfig.load(from: path) == nil, "half a file is ignored")

        let entry = EQEntry(preset: "Many", preamp: 0, filters: (0..<20).map { spec("peak", 100 + Double($0) * 100, 1, 1) } + [spec("wobble", 1, 1, 1)])
        check(EQSettings(entry, rate: 48_000).biquads.count == EQProcessor.maxFilters, "settings keep at most maxFilters and skip unknown types")

        check(EQKey.make(uid: "BuiltInSpeakerDevice", builtIn: true, dataSource: 0x6973_706B) == "BuiltInSpeakerDevice#ispk", "speakers key")
        check(EQKey.make(uid: "BuiltInSpeakerDevice", builtIn: true, dataSource: 0x6864_706E) == "BuiltInSpeakerDevice#hdpn", "jack key")
        check(EQKey.make(uid: "AA-BB:output", builtIn: false, dataSource: nil) == "AA-BB:output", "other devices use the UID")
        check(EQKey.make(uid: "Dock", builtIn: false, dataSource: 0x6973_706B) == "Dock", "only built-in devices add the source")
        check(EQKey.make(uid: "BuiltInHeadphones", builtIn: true, dataSource: nil) == "BuiltInHeadphones", "built-in without a source")
    }

    static func processing() {
        // Preamp: -6.0206 dB halves a level that is well under the limiter.
        let half = EQProcessor(rate: 48_000)
        half.update(EQSettings(preamp: -20 * log10(2.0), biquads: []))
        let halved = run(half, [[Float](repeating: 0.5, count: 64)])[0]
        check(halved.allSatisfy { close(Double($0), 0.25, 1e-6) }, "preamp halves the level")

        // A +3 dB peak at 160 Hz raises a 160 Hz tone by 3 dB (and leaves 4 kHz alone).
        let peak = EQProcessor(rate: 48_000)
        peak.update(EQSettings(preamp: 0, biquads: [biquad("peak", 160, 3, 0.7)]))
        let boosted = run(peak, [sine(160, amplitude: 0.1, frames: 48_000)])[0]
        let gain = 20 * log10(rms(boosted[24_000...]) / rms(sine(160, amplitude: 0.1, frames: 24_000)[...]))
        check(close(gain, 3, 0.01), "cascade gain at the centre is \(gain)")
        let treble = EQProcessor(rate: 48_000)
        treble.update(EQSettings(preamp: 0, biquads: [biquad("peak", 160, 3, 0.7)]))
        let untouched = run(treble, [sine(4000, amplitude: 0.1, frames: 48_000)])[0]
        check(close(rms(untouched[24_000...]), rms(sine(4000, amplitude: 0.1, frames: 24_000)[...]), 0.001), "far from the peak nothing changes")

        // Channels have their own state, and strides are honoured.
        let stereo = EQProcessor(rate: 48_000)
        stereo.update(EQSettings(preamp: -3, biquads: [biquad("lowshelf", 100, 4, 0.7)]))
        let tone = sine(300, amplitude: 0.3, frames: 4800)
        let planar = run(stereo, [tone, [Float](repeating: 0, count: tone.count)])
        check(planar[1].allSatisfy { $0 == 0 } && planar[0].contains { $0 != 0 }, "a silent channel stays silent")
        let mixed = EQProcessor(rate: 48_000)
        mixed.update(EQSettings(preamp: -3, biquads: [biquad("lowshelf", 100, 4, 0.7)]))
        var interleaved = [Float](repeating: 0, count: tone.count * 2)
        for (i, s) in tone.enumerated() { interleaved[i * 2] = s }
        var result = [Float](repeating: 0, count: tone.count * 2)
        interleaved.withUnsafeMutableBufferPointer { input in
            result.withUnsafeMutableBufferPointer { output in
                for c in 0..<2 {
                    mixed.inputs[c] = EQPlane(data: input.baseAddress! + c, stride: 2)
                    mixed.outputs[c] = EQPlane(data: output.baseAddress! + c, stride: 2)
                }
                mixed.process(frames: tone.count, channels: 2)
            }
        }
        check((0..<tone.count).allSatisfy { result[$0 * 2] == planar[0][$0] && result[$0 * 2 + 1] == 0 }, "interleaved equals planar")

        // The limiter: quiet signals pass untouched, nothing ever reaches full scale.
        check(EQProcessor.limiterGain(peak: 0.5) == 1 && EQProcessor.limiterGain(peak: EQProcessor.knee) == 1, "limiter is transparent below the knee")
        var last = 0.0
        for peak in stride(from: 0.0, through: 1000.0, by: 0.37) {
            let out = peak * EQProcessor.limiterGain(peak: peak)
            check(out <= EQProcessor.ceiling + 1e-12 && out >= last - 1e-12, "limiter output rises smoothly and stays under the ceiling at \(peak)")
            last = out
        }
        let loud = EQProcessor(rate: 48_000)
        loud.update(EQSettings(preamp: 12, biquads: [biquad("peak", 100, 12, 0.7)]))
        let blasted = run(loud, [sine(100, amplitude: 1.0, frames: 9600), sine(100, amplitude: -1.0, frames: 9600)])
        check(blasted.allSatisfy { $0.allSatisfy { abs($0) <= 1.0 } }, "a hot chain never exceeds full scale")
        check(blasted[0].contains { abs($0) > 0.9 }, "and still gets loud")

        // Bad input must not poison the filters.
        let bad = EQProcessor(rate: 48_000)
        bad.update(EQSettings(preamp: 0, biquads: [biquad("peak", 1000, 6, 1)]))
        _ = run(bad, [[.nan, .infinity, -.infinity, 0.1]])
        let after = run(bad, [sine(1000, amplitude: 0.1, frames: 480)])[0]
        check(after.allSatisfy { $0.isFinite }, "NaN and infinity are dropped")
    }

    static func handOver() {
        let rate = 48_000.0
        let fade = Int(rate * EQProcessor.fadeSeconds)
        let processor = EQProcessor(rate: rate)
        // Before any settings the audio is untouched (unity gain, no filters).
        let silence = run(processor, [[Float](repeating: 0.5, count: 16)])[0]
        check(silence.allSatisfy { $0 == 0.5 }, "no settings means pass-through")

        processor.update(EQSettings(preamp: 0, biquads: []))
        let level = [Float](repeating: 0.5, count: fade * 2)
        check(run(processor, [level])[0].allSatisfy { $0 == 0.5 }, "unity settings")

        // New settings are crossfaded in: no jump, no overshoot, and they arrive in full.
        let quieter = EQSettings(preamp: -20 * log10(2.0), biquads: [])
        processor.update(quieter)
        processor.update(quieter)
        let faded = run(processor, [level])[0]
        check(close(Double(faded[0]), 0.5, 0.001), "the change starts where the sound was")
        check(zip(faded, faded.dropFirst()).allSatisfy { $1 <= $0 }, "and moves one way only")
        check(faded[fade...].allSatisfy { close(Double($0), 0.25, 1e-6) }, "the new settings arrive after about 30 ms")
        check(close(Double(faded[fade / 2]), 0.375, 0.01), "halfway through the fade the level is halfway")

        // The same settings again change nothing.
        processor.update(quieter)
        check(run(processor, [level])[0].allSatisfy { close(Double($0), 0.25, 1e-6) }, "an unchanged update is ignored")
    }
}
