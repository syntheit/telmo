import Foundation

// The maths behind the system EQ, with no audio devices and no AppKit so tests/run.sh can run it: the `eq.json` the sound popup
// writes, RBJ biquads, and the processor the IOProc calls. Equalizer.swift owns the devices.

/// One filter of an `eq.json` entry. `type` is peak, lowshelf, highshelf or highpass.
struct EQFilterSpec: Decodable, Equatable {
    let type: String
    let freq: Double
    let gain: Double
    let q: Double
}

/// What one output plays through: a preamp and the filters the popup resolved from the preset and the bass nudge.
struct EQEntry: Decodable, Equatable {
    let preset: String
    let preamp: Double
    let filters: [EQFilterSpec]

    /// Nothing to apply, so the device needs no tap at all.
    var isFlat: Bool { filters.isEmpty && preamp == 0 }
}

/// `~/.local/state/telmo/eq.json`, written by the sound popup. A missing or unreadable file means no EQ.
struct EQConfig: Decodable, Equatable {
    var enabled = true
    var devices: [String: EQEntry] = [:]

    init() {}

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try values.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        devices = try values.decodeIfPresent([String: EQEntry].self, forKey: .devices) ?? [:]
    }

    private enum CodingKeys: String, CodingKey { case enabled, devices }

    static let path = (ProcessInfo.processInfo.environment["XDG_STATE_HOME"].flatMap { $0.isEmpty ? nil : $0 }
        ?? NSHomeDirectory() + "/.local/state") + "/telmo/eq.json"

    static func load(from path: String = EQConfig.path) -> EQConfig? {
        guard let data = FileManager.default.contents(atPath: path) else { return nil }
        return try? JSONDecoder().decode(EQConfig.self, from: data)
    }
}

/// How the popup names an output in `eq.json`: its UID, and for a built-in device the data source too ('ispk' speakers,
/// 'hdpn' headphone jack), so the two never share a preset. Matches eq_target in crates/sound/src/backend/macos/mod.rs.
enum EQKey {
    static func make(uid: String, builtIn: Bool, dataSource: UInt32?) -> String {
        guard builtIn, let dataSource else { return uid }
        return uid + "#" + fourCC(dataSource)
    }

    static func fourCC(_ code: UInt32) -> String {
        let bytes = (0..<4).map { UInt8((code >> UInt32(24 - 8 * $0)) & 0xFF) }
        return String(decoding: bytes, as: UTF8.self)
    }
}

/// One biquad as `y = b0 x + z1; z1 = b1 x - a1 y + z2; z2 = b2 x - a2 y`, normalized so a0 is 1.
struct Biquad: Equatable {
    var b0, b1, b2, a1, a2: Double

    /// RBJ cookbook filters. Nil for an unknown type or a frequency the sample rate can't carry.
    init?(_ spec: EQFilterSpec, rate: Double) {
        guard spec.freq > 0, spec.freq < rate / 2, spec.q > 0 else { return nil }
        let a = pow(10, spec.gain / 40)
        let w0 = 2 * Double.pi * spec.freq / rate
        let (cos, sin) = (Foundation.cos(w0), Foundation.sin(w0))
        let alpha = sin / (2 * spec.q)
        let s = 2 * a.squareRoot() * alpha
        let b: (Double, Double, Double), d: (Double, Double, Double)
        switch spec.type {
        case "peak":
            b = (1 + alpha * a, -2 * cos, 1 - alpha * a)
            d = (1 + alpha / a, -2 * cos, 1 - alpha / a)
        case "lowshelf":
            b = (a * ((a + 1) - (a - 1) * cos + s), 2 * a * ((a - 1) - (a + 1) * cos), a * ((a + 1) - (a - 1) * cos - s))
            d = ((a + 1) + (a - 1) * cos + s, -2 * ((a - 1) + (a + 1) * cos), (a + 1) + (a - 1) * cos - s)
        case "highshelf":
            b = (a * ((a + 1) + (a - 1) * cos + s), -2 * a * ((a - 1) + (a + 1) * cos), a * ((a + 1) + (a - 1) * cos - s))
            d = ((a + 1) - (a - 1) * cos + s, 2 * ((a - 1) - (a + 1) * cos), (a + 1) - (a - 1) * cos - s)
        case "highpass":
            b = ((1 + cos) / 2, -(1 + cos), (1 + cos) / 2)
            d = (1 + alpha, -2 * cos, 1 - alpha)
        default:
            return nil
        }
        self.init(b0: b.0 / d.0, b1: b.1 / d.0, b2: b.2 / d.0, a1: d.1 / d.0, a2: d.2 / d.0)
    }

    private init(b0: Double, b1: Double, b2: Double, a1: Double, a2: Double) {
        (self.b0, self.b1, self.b2, self.a1, self.a2) = (b0, b1, b2, a1, a2)
    }
}

/// A preamp and a filter chain for one sample rate.
struct EQSettings: Equatable {
    var preamp: Double // dB
    var biquads: [Biquad]

    init(preamp: Double, biquads: [Biquad]) {
        self.preamp = preamp
        self.biquads = Array(biquads.prefix(EQProcessor.maxFilters))
    }

    init(_ entry: EQEntry, rate: Double) {
        self.init(preamp: entry.preamp, biquads: entry.filters.compactMap { Biquad($0, rate: rate) })
    }
}

/// Where one channel's samples sit in a buffer: sample n is `data[n * stride]`. Interleaved audio has stride = channels.
struct EQPlane {
    var data: UnsafeMutablePointer<Float>?
    var stride = 1
}

/// Runs the chain on the audio thread: preamp, biquads per channel in Double (transposed direct form II), then a stereo-linked
/// soft limiter. Everything is allocated up front; `process` never allocates or waits. New settings are handed over with
/// `update` and crossfaded in over about 30 ms, so a preset change doesn't click.
final class EQProcessor {
    static let maxChannels = 8
    static let maxFilters = 12
    /// The limiter leaves the signal alone below the knee and eases into the ceiling (-0.5 dBFS) above it.
    static let knee = 0.89
    static let ceiling = 0.9441
    static let fadeSeconds = 0.03

    /// The caller points these at the buffers before each `process`.
    let inputs = UnsafeMutablePointer<EQPlane>.allocate(capacity: EQProcessor.maxChannels)
    let outputs = UnsafeMutablePointer<EQPlane>.allocate(capacity: EQProcessor.maxChannels)

    private final class Chain {
        let coefficients = UnsafeMutablePointer<Double>.allocate(capacity: EQProcessor.maxFilters * 5)
        let state = UnsafeMutablePointer<Double>.allocate(capacity: EQProcessor.maxChannels * EQProcessor.maxFilters * 2)
        var count = 0
        var gain = 1.0

        init() {
            coefficients.initialize(repeating: 0, count: EQProcessor.maxFilters * 5)
            state.initialize(repeating: 0, count: EQProcessor.maxChannels * EQProcessor.maxFilters * 2)
        }

        deinit {
            coefficients.deallocate()
            state.deallocate()
        }

        func copy(from other: Chain) {
            coefficients.update(from: other.coefficients, count: EQProcessor.maxFilters * 5)
            state.update(from: other.state, count: EQProcessor.maxChannels * EQProcessor.maxFilters * 2)
            count = other.count
            gain = other.gain
        }

        @inline(__always)
        func run(_ input: Double, channel: Int) -> Double {
            var x = input * gain
            let z = state + channel * EQProcessor.maxFilters * 2
            for i in 0..<count {
                let c = coefficients + i * 5
                let y = c[0] * x + z[2 * i]
                z[2 * i] = c[1] * x - c[3] * y + z[2 * i + 1]
                z[2 * i + 1] = c[2] * x - c[4] * y
                // Keep a decaying tail out of the denormals, which are slow.
                if abs(z[2 * i]) < 1e-30 { z[2 * i] = 0 }
                if abs(z[2 * i + 1]) < 1e-30 { z[2 * i + 1] = 0 }
                x = y
            }
            return x
        }
    }

    private let active = Chain()
    private let fading = Chain()
    private var fadeLeft = 0
    private let fadeFrames: Int
    private var configured = false
    private let scratch = UnsafeMutablePointer<Double>.allocate(capacity: EQProcessor.maxChannels)

    // The hand-over: `update` writes here under the lock; the audio thread only ever tries the lock.
    private var lock = os_unfair_lock()
    private let pendingCoefficients = UnsafeMutablePointer<Double>.allocate(capacity: EQProcessor.maxFilters * 5)
    private var pendingCount = 0
    private var pendingGain = 1.0
    private var pendingSettings: EQSettings?
    private var dirty = false

    init(rate: Double) {
        fadeFrames = max(1, Int(rate * Self.fadeSeconds))
        inputs.initialize(repeating: EQPlane(), count: Self.maxChannels)
        outputs.initialize(repeating: EQPlane(), count: Self.maxChannels)
        scratch.initialize(repeating: 0, count: Self.maxChannels)
        pendingCoefficients.initialize(repeating: 0, count: Self.maxFilters * 5)
    }

    deinit {
        inputs.deallocate()
        outputs.deallocate()
        scratch.deallocate()
        pendingCoefficients.deallocate()
    }

    /// Any thread but the audio thread. The change takes effect at the next buffer.
    func update(_ settings: EQSettings) {
        os_unfair_lock_lock(&lock)
        defer { os_unfair_lock_unlock(&lock) }
        guard settings != pendingSettings else { return }
        pendingSettings = settings
        for (i, q) in settings.biquads.enumerated() {
            pendingCoefficients[i * 5 + 0] = q.b0
            pendingCoefficients[i * 5 + 1] = q.b1
            pendingCoefficients[i * 5 + 2] = q.b2
            pendingCoefficients[i * 5 + 3] = q.a1
            pendingCoefficients[i * 5 + 4] = q.a2
        }
        pendingCount = settings.biquads.count
        pendingGain = pow(10, settings.preamp / 20)
        dirty = true
    }

    /// Audio thread. The settings are taken only if the lock is free; otherwise the next buffer gets them.
    private func adoptPending() {
        guard os_unfair_lock_trylock(&lock) else { return }
        defer { os_unfair_lock_unlock(&lock) }
        guard dirty else { return }
        dirty = false
        if configured {
            fading.copy(from: active)
            fadeLeft = fadeFrames
        }
        active.coefficients.update(from: pendingCoefficients, count: Self.maxFilters * 5)
        active.count = pendingCount
        active.gain = pendingGain
        configured = true
    }

    /// Reads `channels` planes from `inputs`, writes them to `outputs`. Both may be the same memory.
    func process(frames: Int, channels: Int) {
        adoptPending()
        let channels = min(channels, Self.maxChannels)
        for n in 0..<frames {
            var peak = 0.0
            let mix = fadeLeft > 0 ? 1 - Double(fadeLeft) / Double(fadeFrames) : 1
            for c in 0..<channels {
                let plane = inputs[c]
                // A NaN would stay in the filters for good.
                let sample = plane.data.map { Double($0[n * plane.stride]) } ?? 0
                let x = sample.isFinite ? sample : 0
                var y = active.run(x, channel: c)
                if fadeLeft > 0 { y = fading.run(x, channel: c) * (1 - mix) + y * mix }
                scratch[c] = y
                peak = max(peak, abs(y))
            }
            if fadeLeft > 0 { fadeLeft -= 1 }
            let gain = Self.limiterGain(peak: peak)
            for c in 0..<channels {
                let plane = outputs[c]
                plane.data?[n * plane.stride] = Float(scratch[c] * gain)
            }
        }
    }

    /// 1 below the knee; above it the peak is eased toward the ceiling, so nothing a chain produces reaches full scale.
    static func limiterGain(peak: Double) -> Double {
        guard peak > knee else { return 1 }
        let room = ceiling - knee
        return (knee + room * tanh((peak - knee) / room)) / peak
    }
}
