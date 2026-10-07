import AVFoundation
import CoreAudio
import Foundation

/// System audio for the sound module's visualizer and song recognition. The "System Audio Recording" and Microphone
/// grants belong to this app, not to the telmo-sound child, so the child asks us: `audio-stream` (what is playing) or
/// `audio-stream mic` keeps the connection open and we send `ok rate=<hz>\n`, then raw little-endian mono Float32
/// samples until the client goes away. Runs on an IPC worker thread.
enum Audio {
    private static let lock = NSLock()
    private static var busy = Set<String>()

    private protocol Source {
        var rate: Double { get }
        func close()
    }

    static func isAudioCommand(_ line: String) -> Bool { line == "audio-stream" || line == "audio-stream mic" }

    static func stream(command: String, to fd: Int32) {
        let mic = command == "audio-stream mic"
        lock.lock()
        let taken = busy.insert(command).inserted
        lock.unlock()
        guard taken else { send("error busy\n", to: fd); return }
        defer {
            lock.lock()
            busy.remove(command)
            lock.unlock()
        }

        let ring = Ring()
        let source: Source
        do { source = mic ? try Mic(ring: ring) : try Tap(ring: ring) } catch let failure as TapFailure {
            send("error \(failure.message)\n", to: fd)
            return
        } catch {
            send("error Couldn't listen to the \(mic ? "microphone" : "system audio").\n", to: fd)
            return
        }
        defer { source.close() }

        guard send("ok rate=\(Int(source.rate))\n", to: fd) else { return }
        var chunk = [Float]()
        while !clientGone(fd) {
            ring.wait()
            ring.drain(into: &chunk)
            if !chunk.isEmpty, !chunk.withUnsafeBytes({ write(all: $0, to: fd) }) { return }
        }
    }

    @discardableResult
    private static func send(_ text: String, to fd: Int32) -> Bool {
        Array(text.utf8).withUnsafeBytes { write(all: $0, to: fd) }
    }

    private static func write(all bytes: UnsafeRawBufferPointer, to fd: Int32) -> Bool {
        var offset = 0
        while offset < bytes.count {
            let n = Darwin.write(fd, bytes.baseAddress! + offset, bytes.count - offset)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { return false } // closed, or the client stopped reading for the whole send timeout
            offset += n
        }
        return true
    }

    /// The client never sends anything after the command, so any readable state means it hung up.
    private static func clientGone(_ fd: Int32) -> Bool {
        var p = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
        return poll(&p, 1, 0) != 0
    }

    /// Newest samples between the IOProc and the socket writer: about 0.7 s, older ones are dropped.
    private final class Ring {
        private var samples = [Float]()
        private var lock = os_unfair_lock()
        private let signal = DispatchSemaphore(value: 0)
        private static let capacity = 32768

        /// Audio thread: never waits for the lock, a missed chunk is only a hole in a visualizer.
        func add(_ new: UnsafeBufferPointer<Float>) {
            guard os_unfair_lock_trylock(&lock) else { return }
            samples.append(contentsOf: new)
            if samples.count > Self.capacity { samples.removeFirst(samples.count - Self.capacity) }
            os_unfair_lock_unlock(&lock)
            signal.signal()
        }

        func wait() { _ = signal.wait(timeout: .now() + .milliseconds(100)) }

        func drain(into out: inout [Float]) {
            os_unfair_lock_lock(&lock)
            swap(&samples, &out)
            samples.removeAll(keepingCapacity: true)
            os_unfair_lock_unlock(&lock)
        }
    }

    private struct TapFailure: Error { let message: String }

    /// The default microphone through AVAudioEngine. The first attempt makes macOS ask for Microphone access.
    private final class Mic: Source {
        private(set) var rate = 48_000.0
        private let engine = AVAudioEngine()

        init(ring: Ring) throws {
            try Self.authorize()
            let input = engine.inputNode
            let format = input.inputFormat(forBus: 0)
            guard format.sampleRate > 0, format.channelCount > 0 else {
                throw TapFailure(message: "There is no microphone to listen to.")
            }
            rate = format.sampleRate
            input.installTap(onBus: 0, bufferSize: 2048, format: format) { buffer, _ in
                Mic.downmix(buffer, into: ring)
            }
            do { try engine.start() } catch {
                input.removeTap(onBus: 0)
                throw TapFailure(message: "Couldn't listen to the microphone.")
            }
        }

        func close() {
            engine.stop()
            engine.inputNode.removeTap(onBus: 0)
        }

        private static func authorize() throws {
            switch AVCaptureDevice.authorizationStatus(for: .audio) {
            case .authorized: return
            case .notDetermined:
                let answered = DispatchSemaphore(value: 0)
                var granted = false
                AVCaptureDevice.requestAccess(for: .audio) { granted = $0; answered.signal() }
                answered.wait()
                if !granted { throw TapFailure(message: "denied") }
            default: throw TapFailure(message: "denied")
            }
        }

        private static func downmix(_ buffer: AVAudioPCMBuffer, into ring: Ring) {
            guard let channels = buffer.floatChannelData else { return }
            let frames = Int(buffer.frameLength)
            let count = Int(buffer.format.channelCount)
            var mono = [Float](repeating: 0, count: frames)
            for c in 0..<count { for i in 0..<frames { mono[i] += channels[c][i] } }
            let scale = 1 / Float(max(count, 1))
            for i in 0..<frames { mono[i] *= scale }
            mono.withUnsafeBufferPointer { ring.add($0) }
        }
    }

    /// The tap, the aggregate device around it and the running IOProc. `close` undoes them, newest first.
    @available(macOS 14.2, *)
    private final class Tap: Source {
        private(set) var rate = 48_000.0
        private var tap = AudioObjectID(kAudioObjectUnknown)
        private var aggregate = AudioObjectID(kAudioObjectUnknown)
        private var ioProc: AudioDeviceIOProcID?
        private let queue = DispatchQueue(label: "telmo.audio", qos: .userInteractive)

        init(ring: Ring) throws {
            do { try open(ring: ring) } catch {
                close()
                throw error
            }
        }

        private func open(ring: Ring) throws {
            let ours = Self.ownProcessObject().map { [$0] } ?? []
            let description = CATapDescription(stereoGlobalTapButExcludeProcesses: ours)
            description.name = "Telmo visualizer"
            description.isPrivate = true
            description.muteBehavior = .unmuted
            // The first attempt is what makes macOS ask for System Audio Recording.
            guard AudioHardwareCreateProcessTap(description, &tap) == noErr else {
                throw TapFailure(message: "denied")
            }
            var format = AudioStreamBasicDescription()
            var size = UInt32(MemoryLayout.size(ofValue: format))
            var address = Self.address(kAudioTapPropertyFormat)
            if AudioObjectGetPropertyData(tap, &address, 0, nil, &size, &format) == noErr, format.mSampleRate > 0 {
                rate = format.mSampleRate
            }

            guard let output = Self.defaultOutputUID() else {
                throw TapFailure(message: "There is no sound output device to listen to.")
            }
            let aggregateDescription: [String: Any] = [
                kAudioAggregateDeviceUIDKey: "io.telmo.visualizer.\(getpid())",
                kAudioAggregateDeviceNameKey: "Telmo visualizer",
                kAudioAggregateDeviceMainSubDeviceKey: output,
                kAudioAggregateDeviceIsPrivateKey: true,
                kAudioAggregateDeviceIsStackedKey: false,
                kAudioAggregateDeviceTapAutoStartKey: true,
                kAudioAggregateDeviceSubDeviceListKey: [[kAudioSubDeviceUIDKey: output]],
                kAudioAggregateDeviceTapListKey: [[
                    kAudioSubTapUIDKey: description.uuid.uuidString,
                    kAudioSubTapDriftCompensationKey: true,
                ]],
            ]
            guard AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &aggregate) == noErr else {
                throw TapFailure(message: "Couldn't listen to the system audio.")
            }
            let status = AudioDeviceCreateIOProcIDWithBlock(&ioProc, aggregate, queue) { _, input, _, _, _ in
                Tap.downmix(input, into: ring)
            }
            guard status == noErr, AudioDeviceStart(aggregate, ioProc) == noErr else {
                throw TapFailure(message: "Couldn't listen to the system audio.")
            }
        }

        func close() {
            if let ioProc {
                AudioDeviceStop(aggregate, ioProc)
                AudioDeviceDestroyIOProcID(aggregate, ioProc)
                self.ioProc = nil
            }
            if aggregate != kAudioObjectUnknown {
                AudioHardwareDestroyAggregateDevice(aggregate)
                aggregate = AudioObjectID(kAudioObjectUnknown)
            }
            if tap != kAudioObjectUnknown {
                AudioHardwareDestroyProcessTap(tap)
                tap = AudioObjectID(kAudioObjectUnknown)
            }
        }

        /// Mono is all the spectrum needs. Handles interleaved and one-buffer-per-channel layouts.
        private static func downmix(_ list: UnsafePointer<AudioBufferList>, into ring: Ring) {
            let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: list))
            guard let first = buffers.first, first.mData != nil else { return }
            let frames = Int(first.mDataByteSize) / (MemoryLayout<Float>.size * Int(max(first.mNumberChannels, 1)))
            var mono = [Float](repeating: 0, count: frames)
            var channels = 0
            for buffer in buffers {
                guard let data = buffer.mData?.assumingMemoryBound(to: Float.self) else { continue }
                let width = Int(max(buffer.mNumberChannels, 1))
                let count = min(frames, Int(buffer.mDataByteSize) / (MemoryLayout<Float>.size * width))
                for frame in 0..<count { for c in 0..<width { mono[frame] += data[frame * width + c] } }
                channels += width
            }
            guard channels > 0 else { return }
            let scale = 1 / Float(channels)
            for i in 0..<frames { mono[i] *= scale }
            mono.withUnsafeBufferPointer { ring.add($0) }
        }

        private static func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
            AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        }

        private static func defaultOutputUID() -> String? {
            var device = AudioObjectID(kAudioObjectUnknown)
            var size = UInt32(MemoryLayout<AudioObjectID>.size)
            var address = address(kAudioHardwarePropertyDefaultOutputDevice)
            guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device) == noErr,
                  device != kAudioObjectUnknown else { return nil }
            var uid: Unmanaged<CFString>?
            size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
            address = Self.address(kAudioDevicePropertyDeviceUID)
            guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &uid) == noErr else { return nil }
            return uid?.takeRetainedValue() as String?
        }

        /// Core Audio's object for this process, so the tap leaves us out.
        private static func ownProcessObject() -> AudioObjectID? {
            var pid = getpid()
            var object = AudioObjectID(kAudioObjectUnknown)
            var size = UInt32(MemoryLayout<AudioObjectID>.size)
            var address = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
            let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object)
            return status == noErr && object != kAudioObjectUnknown ? object : nil
        }
    }
}
