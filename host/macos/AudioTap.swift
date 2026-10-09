import CoreAudio
import Foundation

/// Core Audio property reads shared by the visualizer (Audio.swift) and the system EQ (Equalizer.swift).
enum HAL {
    static func address(_ selector: AudioObjectPropertySelector, scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }

    static func get<T>(_ object: AudioObjectID, _ at: AudioObjectPropertyAddress, default zero: T) -> T? {
        var value = zero
        var size = UInt32(MemoryLayout<T>.size)
        var at = at
        let status = withUnsafeMutablePointer(to: &value) { AudioObjectGetPropertyData(object, &at, 0, nil, &size, $0) }
        return status == noErr ? value : nil
    }

    static func defaultOutputDevice() -> AudioObjectID? {
        let device = get(AudioObjectID(kAudioObjectSystemObject), address(kAudioHardwarePropertyDefaultOutputDevice), default: AudioObjectID(kAudioObjectUnknown))
        return device == kAudioObjectUnknown ? nil : device
    }

    static func uid(of device: AudioObjectID) -> String? {
        let uid: Unmanaged<CFString>? = get(device, address(kAudioDevicePropertyDeviceUID), default: nil) ?? nil
        return uid?.takeRetainedValue() as String?
    }

    static func defaultOutputUID() -> String? { defaultOutputDevice().flatMap(uid(of:)) }

    static func isBuiltIn(_ device: AudioObjectID) -> Bool {
        get(device, address(kAudioDevicePropertyTransportType), default: UInt32(0)) == kAudioDeviceTransportTypeBuiltIn
    }

    /// The output's current data source as a four-character code: 'ispk' speakers, 'hdpn' headphone jack. Nil when it has none.
    static func dataSource(_ device: AudioObjectID) -> UInt32? {
        get(device, address(kAudioDevicePropertyDataSource, scope: kAudioObjectPropertyScopeOutput), default: UInt32(0))
    }

    static func nominalRate(_ device: AudioObjectID) -> Double? {
        get(device, address(kAudioDevicePropertyNominalSampleRate), default: 0.0).flatMap { $0 > 0 ? $0 : nil }
    }

    /// Core Audio's object for this process, so a tap leaves us out.
    static func ownProcessObject() -> AudioObjectID? {
        var pid = getpid()
        var object = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        var at = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
        let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &at, UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object)
        return status == noErr && object != kAudioObjectUnknown ? object : nil
    }
}

/// A private tap of every process but this one, and a private aggregate device that holds the tap and the real output
/// (as its main sub-device, so both run on the output's clock). `close` undoes them, newest first. The caller adds an IOProc.
@available(macOS 14.2, *)
final class TapAggregate {
    enum Failure: Error {
        /// The tap itself was refused, which is what a missing System Audio Recording grant looks like.
        case denied
        case noOutput
        /// Core Audio doesn't know this process, so a tap couldn't leave it out (see `leaveOutSelf`).
        case unknownSelf
        case aggregate
    }

    private(set) var tap = AudioObjectID(kAudioObjectUnknown)
    private(set) var aggregate = AudioObjectID(kAudioObjectUnknown)
    /// The sample rate of the tapped audio.
    private(set) var tapRate = 48_000.0

    /// `output` is the UID of the device to tap; nil takes the current default output. With `leaveOutSelf` the tap refuses to
    /// exist unless it can exclude this process, which an EQ needs: it would otherwise hear its own output and feed back.
    init(name: String, mute: CATapMuteBehavior, output: String? = nil, leaveOutSelf: Bool = false) throws {
        do { try open(name: name, mute: mute, output: output, leaveOutSelf: leaveOutSelf) } catch {
            close()
            throw error
        }
    }

    private func open(name: String, mute: CATapMuteBehavior, output: String?, leaveOutSelf: Bool) throws {
        let ours = HAL.ownProcessObject().map { [$0] } ?? []
        if leaveOutSelf && ours.isEmpty { throw Failure.unknownSelf }
        let description = CATapDescription(stereoGlobalTapButExcludeProcesses: ours)
        description.name = name
        description.isPrivate = true
        description.muteBehavior = mute
        // The first attempt is what makes macOS ask for System Audio Recording.
        guard AudioHardwareCreateProcessTap(description, &tap) == noErr else { throw Failure.denied }
        if let format: AudioStreamBasicDescription = HAL.get(tap, HAL.address(kAudioTapPropertyFormat), default: AudioStreamBasicDescription()),
           format.mSampleRate > 0 {
            tapRate = format.mSampleRate
        }

        guard let output = output ?? HAL.defaultOutputUID() else { throw Failure.noOutput }
        let aggregateDescription: [String: Any] = [
            kAudioAggregateDeviceUIDKey: "io.telmo.\(name.lowercased().replacingOccurrences(of: " ", with: "-")).\(getpid())",
            kAudioAggregateDeviceNameKey: name,
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
        guard AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &aggregate) == noErr else { throw Failure.aggregate }
    }

    func close() {
        if aggregate != kAudioObjectUnknown {
            AudioHardwareDestroyAggregateDevice(aggregate)
            aggregate = AudioObjectID(kAudioObjectUnknown)
        }
        if tap != kAudioObjectUnknown {
            AudioHardwareDestroyProcessTap(tap)
            tap = AudioObjectID(kAudioObjectUnknown)
        }
    }
}
