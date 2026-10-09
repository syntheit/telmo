import AppKit
import CoreAudio

/// The system-wide EQ for the current default output. The sound popup owns the choices (`eq.json`, see EqualizerDSP.swift);
/// this only runs the audio.
///
/// While an output has filters, a process tap of what the other apps play to that output, with `.mutedWhenTapped`, silences
/// their own output to it (apps playing to other devices are not touched), and a private aggregate device of that tap plus the real output runs one IOProc: it reads the tapped audio,
/// runs it through `EQProcessor` and writes the result to the output. The default output device is never changed, so the volume
/// keys, the menu bar picker and Bluetooth stay as they are, and the device's volume still applies after the EQ.
///
/// With the EQ off, or an output without filters, nothing is tapped and nothing is muted. If this app dies, Core Audio
/// destroys its taps and the sound comes back by itself.
///
/// The visualizer (Audio.swift) keeps its own unmuted tap. Taps are independent, so it still hears the original audio of the
/// other apps, and since both leave this app out neither hears the EQ's output.
///
/// Everything runs on one serial queue. Core Audio calls us back on it when the default output, the device list, the sample
/// rate or the data source changes; those are debounced, and syncing again with nothing changed is harmless.
final class Equalizer {
    private let queue = DispatchQueue(label: "telmo.eq")
    private var session: Session?
    private var watched = AudioObjectID(kAudioObjectUnknown)
    private var debounce: DispatchWorkItem?
    /// Keys `telmo-sound eq-seed` is running for, and how often we have asked per key. A key that never gets an entry
    /// is tried a few times, with growing pauses, and then left alone.
    private var seeding = Set<String>()
    private var seedTries = [String: Int]()
    private static let maxSeedTries = 3
    private var stopped = false
    private var wakeObserver: NSObjectProtocol?

    private lazy var changed: AudioObjectPropertyListenerBlock = { [weak self] _, _ in self?.sync(after: 0.3) }
    private static let systemAddresses = [kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyDevices].map { HAL.address($0) }
    private static let deviceAddresses = [
        HAL.address(kAudioDevicePropertyNominalSampleRate),
        HAL.address(kAudioDevicePropertyDataSource, scope: kAudioObjectPropertyScopeOutput),
    ]

    func start() {
        queue.async { [self] in
            for address in Self.systemAddresses {
                var address = address
                AudioObjectAddPropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, queue, changed)
            }
            sync()
        }
        // A sleep can leave the aggregate running on a dead clock.
        wakeObserver = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: nil) { [weak self] _ in
            self?.queue.asyncAfter(deadline: .now() + 1.5) { self?.rebuild() }
        }
    }

    /// `eq reload`: the popup changed eq.json.
    func reload() { queue.async { [self] in sync() } }

    /// For quitting: lets go of the audio before the app is gone, and makes sure nothing starts it again.
    func stop() {
        if let wakeObserver { NSWorkspace.shared.notificationCenter.removeObserver(wakeObserver) }
        wakeObserver = nil
        queue.sync {
            stopped = true
            debounce?.cancel()
            for address in Self.systemAddresses {
                var address = address
                AudioObjectRemovePropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, queue, changed)
            }
            if watched != kAudioObjectUnknown {
                for address in Self.deviceAddresses {
                    var address = address
                    AudioObjectRemovePropertyListenerBlock(watched, &address, queue, changed)
                }
            }
            watched = AudioObjectID(kAudioObjectUnknown)
            endSession()
        }
    }

    private func sync(after delay: Double) {
        queue.async { [self] in
            debounce?.cancel()
            let work = DispatchWorkItem { [self] in sync() }
            debounce = work
            queue.asyncAfter(deadline: .now() + delay, execute: work)
        }
    }

    private func rebuild() {
        endSession()
        sync()
    }

    /// Makes the audio match eq.json for the current default output: starts, updates or ends the session.
    private func sync() {
        guard !stopped else { return }
        guard let device = HAL.defaultOutputDevice(), let uid = HAL.uid(of: device) else { return endSession() }
        watch(device)
        let key = EQKey.make(uid: uid, builtIn: HAL.isBuiltIn(device), dataSource: HAL.dataSource(device))
        guard let config = EQConfig.load(), config.enabled else { return endSession() }
        guard let entry = config.devices[key] else {
            seed(key)
            return endSession()
        }
        guard !entry.isFlat else { return endSession() }

        let rate = HAL.nominalRate(device)
        if let session, session.key == key, session.device == device, session.deviceRate == rate {
            return session.update(entry)
        }
        endSession()
        do {
            session = try Session(key: key, device: device, uid: uid, deviceRate: rate, entry: entry)
            NSLog("telmo: EQ \(entry.preset) on \(uid)")
        } catch {
            NSLog("telmo: EQ cannot start on \(uid): \(error)")
        }
    }

    private func endSession() {
        guard let session else { return }
        session.close()
        self.session = nil
        NSLog("telmo: EQ off")
    }

    /// Listens for the sample rate and data source of the output we play on, which can change under us.
    private func watch(_ device: AudioObjectID) {
        guard device != watched else { return }
        for address in Self.deviceAddresses {
            var address = address
            if watched != kAudioObjectUnknown { AudioObjectRemovePropertyListenerBlock(watched, &address, queue, changed) }
            AudioObjectAddPropertyListenerBlock(device, &address, queue, changed)
        }
        watched = device
    }

    /// An output the popup hasn't seen has no entry yet. The popup's own binary knows the presets, so it writes the default
    /// ones, and we look again when it is done. A failed run is tried again after a pause, up to `maxSeedTries` times.
    private func seed(_ key: String) {
        let tries = seedTries[key, default: 0]
        guard tries < Self.maxSeedTries, seeding.insert(key).inserted else { return }
        seedTries[key] = tries + 1
        let finished = { [weak self] (succeeded: Bool) in
            guard let self else { return }
            queue.async {
                self.seeding.remove(key)
                if succeeded { return self.sync() }
                self.queue.asyncAfter(deadline: .now() + 5 * Double(tries + 1)) { self.sync() }
            }
        }
        guard let program = ModuleLookup.find("sound") else { return finished(false) }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: program)
        process.arguments = ["eq-seed"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        process.terminationHandler = { finished($0.terminationStatus == 0) }
        do { try process.run() } catch {
            NSLog("telmo: cannot run \(program) eq-seed: \(error)")
            finished(false)
        }
    }

    private struct SessionFailure: Error, CustomStringConvertible {
        let description: String
    }

    /// One tap and aggregate running one IOProc for one output at one sample rate.
    @available(macOS 14.2, *)
    private final class Session {
        let key: String
        let device: AudioObjectID
        let deviceRate: Double?
        private let processor: EQProcessor
        private let rate: Double
        private let tapAggregate: TapAggregate
        private var ioProc: AudioDeviceIOProcID?
        private var stopped = false

        init(key: String, device: AudioObjectID, uid: String, deviceRate: Double?, entry: EQEntry) throws {
            self.key = key
            self.device = device
            self.deviceRate = deviceRate
            do { tapAggregate = try TapAggregate(name: "Telmo EQ", mute: .mutedWhenTapped, output: uid, scoped: true, leaveOutSelf: true) } catch TapAggregate.Failure.denied {
                throw SessionFailure(description: "Allow System Audio Recording for Telmo in System Settings.")
            } catch TapAggregate.Failure.unknownSelf {
                throw SessionFailure(description: "Core Audio does not list Telmo as an audio process, so the EQ would hear itself")
            } catch {
                throw SessionFailure(description: "the audio aggregate could not be created")
            }
            let aggregate = tapAggregate.aggregate
            rate = HAL.nominalRate(aggregate) ?? deviceRate ?? tapAggregate.tapRate
            processor = EQProcessor(rate: rate)
            processor.update(EQSettings(entry, rate: rate))
            do { try start(aggregate) } catch {
                close()
                throw error
            }
        }

        private func start(_ aggregate: AudioObjectID) throws {
            // The IOProc writes raw Float32 samples, which would be noise in any other format.
            guard Self.isFloat32(HAL.get(tapAggregate.tap, HAL.address(kAudioTapPropertyFormat), default: AudioStreamBasicDescription())),
                  Self.outputsAreFloat32(aggregate) else {
                throw SessionFailure(description: "the output does not use 32-bit float samples")
            }
            let proc: AudioDeviceIOProc = { _, _, input, _, output, _, context in
                guard let context else { return noErr }
                Unmanaged<Session>.fromOpaque(context).takeUnretainedValue().render(input, output)
                return noErr
            }
            guard AudioDeviceCreateIOProcID(aggregate, proc, Unmanaged.passUnretained(self).toOpaque(), &ioProc) == noErr,
                  AudioDeviceStart(aggregate, ioProc) == noErr else {
                throw SessionFailure(description: "the audio could not be started")
            }
        }

        func update(_ entry: EQEntry) { processor.update(EQSettings(entry, rate: rate)) }

        /// Stops the IOProc first (this waits for a running call), then the aggregate and the tap, which un-mutes the apps.
        func close() {
            guard !stopped else { return }
            stopped = true
            if let ioProc {
                AudioDeviceStop(tapAggregate.aggregate, ioProc)
                AudioDeviceDestroyIOProcID(tapAggregate.aggregate, ioProc)
                self.ioProc = nil
            }
            tapAggregate.close()
        }

        private static func isFloat32(_ format: AudioStreamBasicDescription?) -> Bool {
            guard let format else { return false }
            return format.mFormatID == kAudioFormatLinearPCM && format.mFormatFlags & kAudioFormatFlagIsFloat != 0 && format.mBitsPerChannel == 32
        }

        private static func outputsAreFloat32(_ device: AudioObjectID) -> Bool {
            var at = HAL.address(kAudioDevicePropertyStreams, scope: kAudioObjectPropertyScopeOutput)
            var size: UInt32 = 0
            guard AudioObjectGetPropertyDataSize(device, &at, 0, nil, &size) == noErr, size > 0 else { return false }
            var streams = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
            guard AudioObjectGetPropertyData(device, &at, 0, nil, &size, &streams) == noErr else { return false }
            return streams.allSatisfy {
                isFloat32(HAL.get($0, HAL.address(kAudioStreamPropertyVirtualFormat), default: AudioStreamBasicDescription()))
            }
        }

        /// Audio thread: no allocation, no locks, no logging. Points the processor at the buffers; each buffer holds
        /// one channel or several interleaved ones.
        func render(_ input: UnsafePointer<AudioBufferList>, _ output: UnsafeMutablePointer<AudioBufferList>) {
            let outBuffers = UnsafeMutableAudioBufferListPointer(output)
            let inBuffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
            for buffer in outBuffers {
                if let data = buffer.mData { memset(data, 0, Int(buffer.mDataByteSize)) }
            }
            let (outChannels, outFrames) = Self.planes(of: outBuffers, into: processor.outputs)
            let (inChannels, inFrames) = Self.planes(of: inBuffers, into: processor.inputs)
            if inChannels > 0, outChannels > 0 {
                processor.process(frames: min(inFrames, outFrames), inChannels: inChannels, outChannels: outChannels)
            }
        }

        private static func planes(of buffers: UnsafeMutableAudioBufferListPointer, into planes: UnsafeMutablePointer<EQPlane>) -> (channels: Int, frames: Int) {
            var channels = 0
            var frames = Int.max
            for buffer in buffers {
                let width = Int(buffer.mNumberChannels)
                guard width > 0, channels + width <= EQProcessor.maxChannels, let data = buffer.mData?.assumingMemoryBound(to: Float.self) else { continue }
                frames = min(frames, Int(buffer.mDataByteSize) / (MemoryLayout<Float>.size * width))
                for c in 0..<width { planes[channels + c] = EQPlane(data: data + c, stride: width) }
                channels += width
            }
            return (channels, channels > 0 ? frames : 0)
        }
    }
}
